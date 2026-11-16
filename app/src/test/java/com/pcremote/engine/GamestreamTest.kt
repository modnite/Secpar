package com.pcremote.engine

import com.pcremote.agent.Msg
import com.pcremote.agent.Wire
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class GamestreamTest {
    private val pin = "a".repeat(64)
    private val otherPin = "b".repeat(64)

    private fun info(
        id: String = "gs1",
        tunnelPort: Int = 51234,
        tunnelPin: String = otherPin,
        addresses: List<String> = listOf("192.168.1.20", "203.0.113.7"),
        certificate: String = pin,
    ) = Msg.GameStreamInfo(id, 46989, 46984, certificate, tunnelPort, tunnelPin, addresses)

    @Test fun `engine messages use the documented fields`() {
        assertEquals("""{"t":"gamestream.info","id":"gs1"}""", Wire.encode(Msg.GameStreamInfoAsk("gs1")))
        assertEquals(
            """{"t":"gamestream.pair","id":"gs2","client_id":"0123456789ABCDEF","certificate_sha256":"$pin","pin":"4821","tunnel_certificate_sha256":"$otherPin","tunnel_candidates":["192.168.1.30:54321"]}""",
            Wire.encode(Msg.GameStreamPair("gs2", Gamestream.MOONLIGHT_CLIENT_ID, pin, "4821", otherPin, listOf("192.168.1.30:54321"))),
        )
        assertEquals("""{"t":"gamestream.paired","id":"gs2"}""", Wire.encode(Msg.GameStreamPaired("gs2")))
        assertEquals(
            """{"t":"gamestream.error","id":"gs2","code":"busy","msg":"A pairing is already in progress"}""",
            Wire.encode(Msg.GameStreamError("gs2", "busy", "A pairing is already in progress")),
        )
        assertEquals(
            info(),
            Wire.decode(
                """{"t":"gamestream.info","id":"gs1","http_port":46989,"https_port":46984,"certificate_sha256":"$pin",""" +
                    """"tunnel_port":51234,"tunnel_certificate_sha256":"$otherPin","tunnel_addresses":["192.168.1.20","203.0.113.7"]}""",
            ),
        )
    }

    @Test fun `pins are lowercase hex only`() {
        assertEquals(pin, Gamestream.fingerprint(pin.uppercase()))
        assertEquals(pin, Gamestream.fingerprint("  $pin  "))
        assertNull(Gamestream.fingerprint("a".repeat(63)))
        assertNull(Gamestream.fingerprint("g".repeat(64)))
        assertNull(Gamestream.fingerprint(""))
    }

    @Test fun `only IP literals name a tunnel address`() {
        assertEquals("192.168.1.20", Gamestream.address("192.168.1.20"))
        assertTrue(Gamestream.address("2001:db8::1")!!.contains(':'))
        assertNull(Gamestream.address("pc.example"))
        assertNull(Gamestream.address(""))
    }

    @Test fun `candidates are ip and port`() {
        assertEquals("198.51.100.8:54321", Gamestream.candidate("198.51.100.8:54321"))
        assertEquals("[${Gamestream.address("2001:db8::1")}]:54321", Gamestream.candidate("[2001:db8::1]:54321"))
        assertNull(Gamestream.candidate("198.51.100.8"))
        assertNull(Gamestream.candidate("198.51.100.8:0"))
        assertNull(Gamestream.candidate("198.51.100.8:70000"))
        assertNull(Gamestream.candidate("pc.example:54321"))
        assertEquals(listOf("192.168.1.30:54321"), Gamestream.candidates(listOf("192.168.1.30:54321", "192.168.1.30:54321", "garbage")))
    }

    @Test fun `a full answer gives both ports, the pin and the tunnel`() {
        val endpoint = Gamestream.read(info()).getOrThrow()
        assertEquals(46989, endpoint.httpPort)
        assertEquals(46984, endpoint.httpsPort)
        assertEquals(pin, endpoint.certificate)
        assertTrue(endpoint.tunnelReady)
        assertEquals(51234, endpoint.tunnelPort)
        assertEquals(listOf("192.168.1.20", "203.0.113.7"), endpoint.tunnelAddresses)
    }

    @Test fun `an answer without a tunnel is usable but has nowhere to dial`() {
        val endpoint = Gamestream.read(Msg.GameStreamInfo("gs1", 46989, 46984, pin)).getOrThrow()
        assertFalse(endpoint.tunnelReady)
        assertEquals(0, endpoint.tunnelPort)
    }

    @Test fun `half a tunnel, a bad pin or a name is refused whole`() {
        assertTrue(Gamestream.read(info(tunnelPort = 0)).isFailure)
        assertTrue(Gamestream.read(info(certificate = "")).isFailure)
        assertTrue(Gamestream.read(info(tunnelPin = "nope")).isFailure)
        assertTrue(Gamestream.read(info(addresses = listOf("pc.example"))).isFailure)
        assertTrue(Gamestream.read(info(addresses = emptyList())).isFailure)
        val bad = Gamestream.read(Msg.GameStreamInfo("gs1", 0, 46984, pin)).exceptionOrNull()
        assertTrue(bad is GamestreamProblem)
        assertTrue((bad as GamestreamProblem).message.orEmpty().endsWith("."))
    }

    @Test fun `the pair request carries the moonlight id and both pins`() {
        val request = Gamestream.pairRequest("gs2", pin.uppercase(), "4821", otherPin, listOf("198.51.100.8:54321"))
            .getOrThrow()
        assertEquals(Gamestream.MOONLIGHT_CLIENT_ID, request.client_id)
        assertEquals(pin, request.certificate_sha256)
        assertEquals("4821", request.pin)
        assertEquals(otherPin, request.tunnel_certificate_sha256)
        assertEquals(listOf("198.51.100.8:54321"), request.tunnel_candidates)
    }

    @Test fun `a pair request that cannot be trusted is not sent`() {
        assertTrue(Gamestream.pairRequest("gs2", pin, "482", otherPin, emptyList()).isFailure)
        assertTrue(Gamestream.pairRequest("gs2", pin, "48a1", otherPin, emptyList()).isFailure)
        assertTrue(Gamestream.pairRequest("gs2", "", "4821", otherPin, emptyList()).isFailure)
        assertTrue(Gamestream.pairRequest("gs2", pin, "4821", "", emptyList()).isFailure)
        assertTrue(Gamestream.pairRequest("bad id", pin, "4821", otherPin, emptyList()).isFailure)
        assertTrue(Gamestream.pairRequest("x".repeat(65), pin, "4821", otherPin, emptyList()).isFailure)
    }

    @Test fun `host words are shown plainly and known codes get a sentence`() {
        assertEquals("A pairing is already in progress", Gamestream.problem("busy", "A pairing is already in progress"))
        assertEquals("That computer cannot start a stream right now.", Gamestream.problem("unavailable"))
        assertEquals("That computer did not answer.", Gamestream.problem("weird", "  "))
        assertEquals("That computer did not accept this device.", Gamestream.problem("pair_failed", "null"))
    }

    @Test fun `info answers are matched by id`() = runTest {
        val incoming = MutableSharedFlow<Msg>(extraBufferCapacity = 8)
        val asked = ArrayList<Msg>()
        val client = GamestreamClient({ m ->
            asked += m
            val id = (m as Msg.GameStreamInfoAsk).id
            incoming.tryEmit(info(id = "other"))
            incoming.tryEmit(info(id = id))
            true
        }, incoming)
        val endpoint = client.info().getOrThrow()
        assertEquals(46989, endpoint.httpPort)
        assertTrue(asked.single() is Msg.GameStreamInfoAsk)
    }

    @Test fun `a refused request reads as a sentence`() = runTest {
        val incoming = MutableSharedFlow<Msg>(extraBufferCapacity = 8)
        val client = GamestreamClient({ m ->
            incoming.tryEmit(Msg.GameStreamError((m as Msg.GameStreamInfoAsk).id, "unavailable"))
            true
        }, incoming)
        assertEquals("That computer cannot start a stream right now.", client.info().exceptionOrNull()?.message)
    }

    @Test fun `a quiet computer is not waited on forever`() = runTest {
        val incoming = MutableSharedFlow<Msg>(extraBufferCapacity = 8)
        val client = GamestreamClient({ true }, incoming)
        assertEquals("That computer did not answer.", client.info().exceptionOrNull()?.message)
    }

    @Test fun `a closed connection says so`() = runTest {
        val incoming = MutableSharedFlow<Msg>(extraBufferCapacity = 8)
        val client = GamestreamClient({ false }, incoming)
        assertEquals("The connection to that computer is closed.", client.info().exceptionOrNull()?.message)
    }

    @Test fun `pairing waits for paired and reports a refusal`() = runTest {
        val incoming = MutableSharedFlow<Msg>(extraBufferCapacity = 8)
        val client = GamestreamClient({ m ->
            val pair = m as Msg.GameStreamPair
            incoming.tryEmit(Msg.GameStreamPaired(pair.id))
            true
        }, incoming)
        assertTrue(client.pair(pin, "4821", otherPin, listOf("192.168.1.30:54321")).isSuccess)

        val refusing = GamestreamClient({ m ->
            incoming.tryEmit(Msg.GameStreamError((m as Msg.GameStreamPair).id, "pair_failed"))
            true
        }, incoming)
        assertEquals("That computer did not accept this device.", refusing.pair(pin, "4821", otherPin, emptyList()).exceptionOrNull()?.message)
    }
}
