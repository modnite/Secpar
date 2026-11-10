package com.pcremote.guest

import android.view.KeyEvent
import android.view.MotionEvent
import com.pcremote.account.GuestGrant
import com.pcremote.agent.AgentState
import com.pcremote.agent.Msg
import com.pcremote.agent.PcInfo
import com.pcremote.agent.Wire
import com.pcremote.agent.Ws
import com.pcremote.agent.WsFactory
import com.pcremote.agent.WsListener
import com.pcremote.modes.PadButton
import com.pcremote.modes.PadState
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class GuestRelayTest {
    @Test fun `an https origin becomes the guest socket url`() {
        assertEquals("wss://relay.example.com/v1/guest?pc=pc_1-A", GuestRelay.socketUrl("https://relay.example.com", "pc_1-A"))
        assertEquals("wss://relay.example.com:8443/v1/guest?pc=abc", GuestRelay.socketUrl("https://relay.example.com:8443/", "abc"))
    }

    @Test fun `anything besides scheme host and port is refused`() {
        for (relay in listOf("http://relay.example.com", "wss://relay.example.com", "https://relay.example.com/v1/phone", "https://relay.example.com?x=1",
            "https://user@relay.example.com", "https://relay.example.com#a", "https://relay.example.com:99999", "https://relay.example.com:0",
            "https://relay.example.com\\evil", "https:// relay.example.com", "https://[::1]", "https://", "relay.example.com", "",
            "https://relay.example.com/%2e%2e", "https://-bad.example.com", "https://relay.example.com@evil.test", "https://a_b.example.com"))
            assertNull(relay, GuestRelay.socketUrl(relay, "abc"))
    }

    @Test fun `a pc id cannot carry a path or a query`() {
        for (pc in listOf("", "a/b", "a?x=1", "a&pc=b", "a b", "a#b", "a%2fb", "x".repeat(65), "a\nb", "a.b"))
            assertNull(pc, GuestRelay.socketUrl("https://relay.example.com", pc))
        assertNotNull(GuestRelay.socketUrl("https://relay.example.com", "x".repeat(64)))
    }

    @Test fun `host error codes read as plain messages`() {
        assertEquals("The computer is busy. Try again later.", GuestMessages.forCode("busy"))
        assertEquals("The computer is locked.", GuestMessages.forCode("locked"))
        assertEquals("The owner does not allow that.", GuestMessages.forCode("forbidden"))
        assertEquals("The computer did not accept the request. Ask again.", GuestMessages.forCode("guest_refused"))
        assertEquals("The owner ended the session.", GuestMessages.forCode("guest_ended"))
        assertEquals("The owner ended the session: Back in a minute", GuestMessages.forCode("guest_ended", "Back in a minute"))
        assertEquals("The owner ended the session.", GuestMessages.forCode("guest_ended", "x".repeat(200)))
        assertEquals("The computer refused the connection.", GuestMessages.forCode("anything_else"))
        assertTrue(GuestMessages.ends("guest_ended"))
        assertFalse(GuestMessages.ends("forbidden"))
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class GuestConnectorTest {
    private class FakeRelay(val reply: (Msg) -> List<Msg>) : WsFactory {
        var url: String? = null
        val sent = mutableListOf<String>()
        var closed = false
        lateinit var listener: WsListener
        override fun open(host: String, port: Int, fingerprint: String, listener: WsListener): Ws {
            this.listener = listener
            listener.onOpen()
            return object : Ws {
                override fun send(text: String): Boolean {
                    sent += text
                    Wire.decode(text)?.let { m -> reply(m).forEach { listener.onText(Wire.encode(it)) } }
                    return true
                }
                override fun send(bytes: ByteArray) = true
                override fun close() { closed = true }
            }
        }
    }
    private val grant = GuestGrant("aaa.bbb.ccc", 1060, "pc_1", "https://relay.example.com", null)
    private val welcome = Msg.Welcome(1, PcInfo("pc_1", "Studio PC"), listOf("stream", "input", "pad"))

    @Test fun `the first frame is the grant and the route is registered after welcome`() = runTest {
        val relay = FakeRelay { listOf(welcome) }
        val route = GuestConnector.open(backgroundScope, grant, "Studio PC", "bob_1", setOf("view", "gamepad"), "r1") { url -> relay.url = url; relay }
        assertEquals("wss://relay.example.com/v1/guest?pc=pc_1", relay.url)
        assertEquals("""{"t":"guest","grant":"aaa.bbb.ccc"}""", relay.sent.first())
        assertEquals(1, relay.sent.size)
        assertEquals("guest:r1", route)
        val link = GuestSessions.find(route)!!
        assertTrue(GuestSessions.isGuest(route))
        assertTrue(GuestSessions.allows(route, "gamepad"))
        assertFalse(GuestSessions.allows(route, "audio"))
        assertEquals("Studio PC", link.pc.name)
        assertEquals("bob_1", link.ownerHandle)
        assertTrue(link.pc.allHosts.isEmpty() && link.pc.relay == null)
        assertEquals("@relay", link.conn.connectedHost)
        assertEquals(setOf("stream", "input", "pad"), (link.conn.state.value as AgentState.Online).caps)
        link.conn.close(); GuestSessions.remove(route)
        assertNull(GuestSessions.find(route))
    }

    @Test fun `a host error becomes a plain message and nothing is registered`() = runTest {
        for ((code, text) in listOf("busy" to "The computer is busy. Try again later.", "locked" to "The computer is locked.",
            "guest_refused" to "The computer did not accept the request. Ask again.")) {
            val relay = FakeRelay { listOf(Msg.Error(code)) }
            try {
                GuestConnector.open(backgroundScope, grant, "Studio PC", "bob_1", setOf("view"), "r2") { relay }
                fail(code)
            } catch (e: GuestException) { assertEquals(text, e.message) }
            assertNull(GuestSessions.find("guest:r2"))
            assertTrue(relay.closed)
        }
    }

    @Test fun `a relay origin with a path is refused before any socket opens`() = runTest {
        var opened = false
        try {
            GuestConnector.open(backgroundScope, GuestGrant("g", 1, "pc_1", "https://relay.example.com/x", null), "n", "o", setOf("view"), "r3") { opened = true; FakeRelay { emptyList() } }
            fail()
        } catch (e: GuestException) { assertEquals("Unexpected answer from Secpar.", e.message) }
        assertFalse(opened)
    }

    @Test fun `a dropped friend socket ends the connection instead of reconnecting`() = runTest {
        val relay = FakeRelay { listOf(welcome) }
        val route = GuestConnector.open(backgroundScope, grant, "n", "o", setOf("view"), "r4") { relay }
        val conn = GuestSessions.find(route)!!.conn
        relay.listener.onFailure(java.io.EOFException("gone"), false)
        assertEquals(AgentState.Closed, conn.state.value)
        assertEquals(1, relay.sent.size)
        conn.close(); GuestSessions.remove(route)
    }

    @Test fun `the guest frame hides the grant in logs and encodes as the contract says`() {
        assertEquals("""{"t":"guest","grant":"x.y.z"}""", Wire.encode(Msg.Guest("x.y.z")))
        assertFalse(Msg.Guest("x.y.z").toString().contains("x.y.z"))
        val w = Wire.decode("""{"t":"welcome","pc":{"id":"p","name":"n"},"caps":["pad"]}""") as Msg.Welcome
        assertEquals(listOf("pad"), w.caps)
    }
}
