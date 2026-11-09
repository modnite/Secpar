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

class PadMapperTest {
    private val xbox = mapOf(
        MotionEvent.AXIS_X to (-1f to 1f), MotionEvent.AXIS_Y to (-1f to 1f), MotionEvent.AXIS_Z to (-1f to 1f), MotionEvent.AXIS_RZ to (-1f to 1f),
        MotionEvent.AXIS_LTRIGGER to (0f to 1f), MotionEvent.AXIS_RTRIGGER to (0f to 1f), MotionEvent.AXIS_HAT_X to (-1f to 1f), MotionEvent.AXIS_HAT_Y to (-1f to 1f))

    private fun PadMapper.set(vararg v: Pair<Int, Float>) { val m = v.toMap(); axes { m[it] ?: 0f } }

    @Test fun `face shoulder and centre buttons map to the XInput mask`() {
        val m = PadMapper(xbox)
        val pairs = mapOf(KeyEvent.KEYCODE_BUTTON_A to 0x1000, KeyEvent.KEYCODE_BUTTON_B to 0x2000, KeyEvent.KEYCODE_BUTTON_X to 0x4000,
            KeyEvent.KEYCODE_BUTTON_Y to 0x8000, KeyEvent.KEYCODE_BUTTON_L1 to 0x0100, KeyEvent.KEYCODE_BUTTON_R1 to 0x0200,
            KeyEvent.KEYCODE_BUTTON_THUMBL to 0x0040, KeyEvent.KEYCODE_BUTTON_THUMBR to 0x0080, KeyEvent.KEYCODE_BUTTON_START to 0x0010,
            KeyEvent.KEYCODE_BUTTON_SELECT to 0x0020, KeyEvent.KEYCODE_BUTTON_MODE to 0x0400)
        for ((code, mask) in pairs) {
            assertTrue(m.key(code, true))
            assertEquals(code.toString(), mask, m.state().btn)
            m.key(code, false)
            assertEquals(0, m.state().btn)
        }
        m.key(KeyEvent.KEYCODE_BUTTON_A, true); m.key(KeyEvent.KEYCODE_BUTTON_Y, true)
        assertEquals(0x9000, m.state().btn)
    }

    @Test fun `keys that are not controller buttons are left alone`() {
        val m = PadMapper(xbox)
        assertFalse(m.key(KeyEvent.KEYCODE_BACK, true))
        assertFalse(m.key(KeyEvent.KEYCODE_A, true))
        assertFalse(m.key(KeyEvent.KEYCODE_BUTTON_C, true))
        assertEquals(PadState(), m.state())
    }

    @Test fun `sticks use XInput up positive and the full 16 bit range`() {
        val m = PadMapper(xbox)
        m.set(MotionEvent.AXIS_X to 1f, MotionEvent.AXIS_Y to -1f, MotionEvent.AXIS_Z to -1f, MotionEvent.AXIS_RZ to 1f)
        val s = m.state()
        assertEquals(32767, s.lx); assertEquals(32767, s.ly)
        assertEquals(-32768, s.rx); assertEquals(-32768, s.ry)
        m.set(MotionEvent.AXIS_X to -1f, MotionEvent.AXIS_Y to 1f)
        assertEquals(-32768, m.state().lx); assertEquals(-32768, m.state().ly)
    }

    @Test fun `drift inside the dead zone is silent and the rest of the travel is stretched`() {
        val m = PadMapper(xbox)
        m.set(MotionEvent.AXIS_X to 0.05f, MotionEvent.AXIS_Y to -0.05f)
        assertEquals(0, m.state().lx); assertEquals(0, m.state().ly)
        m.set(MotionEvent.AXIS_X to 0.5f)
        val lx = m.state().lx
        assertTrue("stretched $lx", lx in 14000..15500)
        m.set(MotionEvent.AXIS_X to 0.08f)
        assertEquals(0, m.state().lx)
        val (x, y) = PadMapper.deadZone(0.6f, 0.8f)
        assertEquals(0.6f / 1f * ((1f - 0.08f) / 0.92f), x, 1e-5f); assertEquals(0.8f, y, 1e-5f)
    }

    @Test fun `triggers read 0 to 255 and a small pull is ignored`() {
        val m = PadMapper(xbox)
        m.set(MotionEvent.AXIS_LTRIGGER to 0.5f, MotionEvent.AXIS_RTRIGGER to 1f)
        assertEquals(127, m.state().lt); assertEquals(255, m.state().rt)
        m.set(MotionEvent.AXIS_LTRIGGER to 0.03f)
        assertEquals(0, m.state().lt)
    }

    @Test fun `the d-pad comes from the hat or from d-pad keys`() {
        val m = PadMapper(xbox)
        m.set(MotionEvent.AXIS_HAT_X to -1f)
        assertEquals(PadButton.LEFT.mask, m.state().btn)
        m.set(MotionEvent.AXIS_HAT_X to 1f, MotionEvent.AXIS_HAT_Y to -1f)
        assertEquals(PadButton.RIGHT.mask or PadButton.UP.mask, m.state().btn)
        m.set(MotionEvent.AXIS_HAT_Y to 1f)
        assertEquals(PadButton.DOWN.mask, m.state().btn)
        m.set()
        assertEquals(0, m.state().btn)
        m.key(KeyEvent.KEYCODE_DPAD_UP, true); m.key(KeyEvent.KEYCODE_DPAD_LEFT, true)
        assertEquals(PadButton.UP.mask or PadButton.LEFT.mask, m.state().btn)
    }

    @Test fun `a digital trigger button counts as a full pull`() {
        val m = PadMapper(mapOf(MotionEvent.AXIS_X to (-1f to 1f), MotionEvent.AXIS_Y to (-1f to 1f)))
        m.key(KeyEvent.KEYCODE_BUTTON_L2, true)
        assertEquals(255, m.state().lt); assertEquals(0, m.state().rt)
        m.key(KeyEvent.KEYCODE_BUTTON_L2, false)
        assertEquals(0, m.state().lt)
    }

    @Test fun `an older DualShock layout puts the triggers on RX and RY and the right stick on Z and RZ`() {
        val ds4 = mapOf(MotionEvent.AXIS_X to (-1f to 1f), MotionEvent.AXIS_Y to (-1f to 1f), MotionEvent.AXIS_Z to (-1f to 1f),
            MotionEvent.AXIS_RZ to (-1f to 1f), MotionEvent.AXIS_RX to (-1f to 1f), MotionEvent.AXIS_RY to (-1f to 1f))
        val m = PadMapper(ds4)
        m.set(MotionEvent.AXIS_RX to -1f, MotionEvent.AXIS_RY to 1f, MotionEvent.AXIS_Z to 1f)
        val s = m.state()
        assertEquals(0, s.lt); assertEquals(255, s.rt); assertEquals(32767, s.rx)
        m.set(MotionEvent.AXIS_RX to 0f, MotionEvent.AXIS_RY to 0f)
        assertEquals("a half pull on a -1..1 trigger", 127, m.state().lt)
    }

    @Test fun `Z and RZ that rest at zero with RX and RY present are the triggers`() {
        val pad = mapOf(MotionEvent.AXIS_X to (-1f to 1f), MotionEvent.AXIS_Y to (-1f to 1f), MotionEvent.AXIS_Z to (0f to 1f),
            MotionEvent.AXIS_RZ to (0f to 1f), MotionEvent.AXIS_RX to (-1f to 1f), MotionEvent.AXIS_RY to (-1f to 1f))
        val m = PadMapper(pad)
        m.set(MotionEvent.AXIS_Z to 1f, MotionEvent.AXIS_RX to 1f)
        assertEquals(255, m.state().lt); assertEquals(32767, m.state().rx)
    }

    @Test fun `brake and gas are triggers and RX RY the right stick`() {
        val pad = mapOf(MotionEvent.AXIS_X to (-1f to 1f), MotionEvent.AXIS_Y to (-1f to 1f), MotionEvent.AXIS_RX to (-1f to 1f),
            MotionEvent.AXIS_RY to (-1f to 1f), MotionEvent.AXIS_BRAKE to (0f to 1f), MotionEvent.AXIS_GAS to (0f to 1f))
        val m = PadMapper(pad)
        m.set(MotionEvent.AXIS_BRAKE to 1f, MotionEvent.AXIS_GAS to 0.5f, MotionEvent.AXIS_RX to -1f)
        val s = m.state()
        assertEquals(255, s.lt); assertEquals(127, s.rt); assertEquals(-32768, s.rx)
    }

    @Test fun `release clears every button and axis`() {
        val m = PadMapper(xbox)
        m.key(KeyEvent.KEYCODE_BUTTON_A, true); m.set(MotionEvent.AXIS_X to 1f)
        m.release()
        assertEquals(PadState(), m.state())
    }

    @Test fun `the message carries the id and every field`() {
        val m = PadMapper(xbox)
        m.key(KeyEvent.KEYCODE_BUTTON_A, true); m.set(MotionEvent.AXIS_RTRIGGER to 1f)
        assertEquals("""{"t":"pad","id":2,"btn":4096,"lt":0,"rt":255,"lx":0,"ly":0,"rx":0,"ry":0}""", Wire.encode(m.state().toMsg(2)))
        assertEquals("""{"t":"pad.off","id":2}""", Wire.encode(Msg.PadOff(2)))
    }
}

class PadIdsTest {
    @Test fun `ids go out in plug in order`() {
        val ids = PadIds()
        assertEquals(0, ids.attach(10, "a")); assertEquals(1, ids.attach(11, "b")); assertEquals(2, ids.attach(12, "c")); assertEquals(3, ids.attach(13, "d"))
        assertNull(ids.attach(14, "e"))
        assertEquals(1, ids.idOf(11))
    }

    @Test fun `a controller that comes back gets its old id`() {
        val ids = PadIds()
        ids.attach(10, "a"); ids.attach(11, "b"); ids.attach(12, "c")
        assertEquals(0, ids.detach(10))
        assertEquals(1, ids.detach(11))
        // a new pad does not take an id someone else may come back to
        assertEquals(3, ids.attach(20, "new"))
        assertEquals(1, ids.attach(31, "b"))
        assertEquals(0, ids.attach(30, "a"))
    }

    @Test fun `a returning controller whose id was taken gets another one`() {
        val ids = PadIds()
        ids.attach(1, "a"); ids.attach(2, "b"); ids.attach(3, "c"); ids.attach(4, "d")
        ids.detach(1)
        assertEquals(0, ids.attach(5, "e"))   // all ids are held or reserved, so the free one is used
        assertNull(ids.attach(6, "a"))        // none free
        ids.detach(2)
        assertEquals(1, ids.attach(7, "a"))
    }

    @Test fun `two identical pads are two controllers`() {
        val ids = PadIds()
        assertEquals(0, ids.attach(1, "same")); assertEquals(1, ids.attach(2, "same"))
        assertEquals(0, ids.detach(1))
        assertEquals(0, ids.attach(3, "same"))
    }

    @Test fun `unplugging an unknown device does nothing and the same device is not added twice`() {
        val ids = PadIds()
        assertNull(ids.detach(99))
        assertEquals(0, ids.attach(1, "a")); assertEquals(0, ids.attach(1, "a"))
        assertEquals(setOf(1), ids.devices())
        assertEquals(1, ids.deviceOf(0))
    }
}

class PadFeedTest {
    private val a = PadState(btn = 0x1000)
    private val b = PadState(btn = 0x1000, lx = 5)

    @Test fun `the first state goes out at once and an unchanged state does not`() {
        val f = PadFeed()
        assertEquals(a.toMsg(0), f.offer(0, a, 100))
        assertNull(f.offer(0, a, 200))
    }

    @Test fun `a change inside the minimum gap is held and the newest one wins`() {
        val f = PadFeed(minMs = 8)
        f.offer(0, a, 100)
        assertNull(f.offer(0, b, 103))
        assertNull(f.offer(0, b.copy(lx = 9), 105))
        assertTrue(f.tick(106).isEmpty())
        assertEquals(listOf(b.copy(lx = 9).toMsg(0)), f.tick(108))
        assertTrue(f.tick(109).isEmpty())
    }

    @Test fun `a held change that returns to the sent state sends nothing`() {
        val f = PadFeed(minMs = 8)
        f.offer(0, a, 100); f.offer(0, b, 102); assertNull(f.offer(0, a, 104))
        assertTrue(f.tick(120).isEmpty())
    }

    @Test fun `an idle controller repeats its state once a second`() {
        val f = PadFeed()
        f.offer(0, a, 0)
        assertTrue(f.tick(500).isEmpty())
        assertEquals(listOf(a.toMsg(0)), f.tick(1000))
        assertTrue(f.tick(1500).isEmpty())
        assertEquals(listOf(a.toMsg(0)), f.tick(2000))
    }

    @Test fun `each controller has its own timer and a dropped one stops`() {
        val f = PadFeed()
        f.offer(0, a, 0); f.offer(1, b, 400)
        assertEquals(listOf(a.toMsg(0)), f.tick(1000))
        assertEquals(listOf(b.toMsg(1)), f.tick(1400))
        f.drop(1)
        assertTrue(f.tick(5000).none { it.id == 1 } && f.tick(6000).all { it.id == 0 })
    }
}

class RumbleTest {
    @Test fun `two motors get low and high and one motor gets the larger`() {
        assertEquals(listOf(10, 200), Rumble.amplitudes(10, 200, 2).toList())
        assertEquals(listOf(200), Rumble.amplitudes(10, 200, 1).toList())
        assertEquals(listOf(0, 0, 0), Rumble.amplitudes(0, 0, 3).toList())
        assertEquals(listOf(255, 0), Rumble.amplitudes(999, -5, 2).toList())
        assertTrue(Rumble.amplitudes(1, 1, 0).isEmpty())
    }
}
