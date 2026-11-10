package com.pcremote.modes

import android.view.KeyEvent
import com.pcremote.agent.Msg
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PhysicalPadsTest {
    private val sent = mutableListOf<Msg>()
    private fun pads(firstId: Int = 0) = PhysicalPads({ sent.add(it) }, firstId)
    private fun pad(i: Int) = sent.filterIsInstance<Msg.Pad>().filter { it.id == i }

    @Test fun `controller keys map to xinput buttons`() {
        val p = pads()
        assertTrue(p.key(5, KeyEvent.KEYCODE_BUTTON_A, true))
        assertTrue(p.key(5, KeyEvent.KEYCODE_BUTTON_L1, true))
        assertTrue(p.key(5, KeyEvent.KEYCODE_DPAD_UP, true))
        p.flush(1000)
        assertEquals(Msg.Pad(0, 0x1000 or 0x0100 or 0x0001, 0, 0, 0, 0, 0, 0), sent.last())
        p.key(5, KeyEvent.KEYCODE_BUTTON_A, false)
        p.flush(1010)
        assertEquals(0x0100 or 0x0001, (sent.last() as Msg.Pad).btn)
    }

    @Test fun `other keys are not taken`() {
        val p = pads()
        assertFalse(p.key(5, KeyEvent.KEYCODE_A, true))
        assertFalse(p.key(5, KeyEvent.KEYCODE_BACK, true))
        assertEquals(0, p.count)
    }

    @Test fun `only changes are sent and a held state is refreshed`() {
        val p = pads()
        p.key(1, KeyEvent.KEYCODE_BUTTON_X, true)
        p.flush(0)
        assertEquals(1, sent.size)
        p.flush(5); p.flush(20)
        assertEquals("nothing changed yet", 1, sent.size)
        p.flush(45); p.flush(90)
        assertEquals("two quick repeats", 3, sent.size)
        p.flush(200)
        assertEquals(3, sent.size)
        p.flush(340)
        assertEquals("held state refresh", 4, sent.size)
        p.key(1, KeyEvent.KEYCODE_BUTTON_X, false)
        p.flush(345)
        assertEquals("change is rate limited to 8 ms", 4, sent.size)
        p.flush(350)
        assertEquals(5, sent.size)
        assertEquals(0, (sent.last() as Msg.Pad).btn)
    }

    @Test fun `sticks triggers and hat`() {
        val p = pads()
        p.axes(3, PadAxes(lx = 1f, ly = -1f, rx = -1f, ry = 1f, lt = 1f, rt = 0.5f, hatX = -1, hatY = 1))
        p.flush(0)
        val m = sent.last() as Msg.Pad
        assertEquals(32767, m.lx)
        assertEquals(32767, m.ly)   // screen up is XInput up
        assertEquals(-32768, m.rx)
        assertEquals(-32768, m.ry)
        assertEquals(255, m.lt)
        assertEquals(127, m.rt)
        assertEquals(0x0004 or 0x0002, m.btn) // left + down
    }

    @Test fun `dead zone keeps centre quiet and full range reachable`() {
        assertEquals(0f to 0f, PhysicalPads.deadZone(0.05f, 0.05f))
        val (x, y) = PhysicalPads.deadZone(1f, 0f)
        assertEquals(1f, x, 0.0001f); assertEquals(0f, y, 0.0001f)
        val (a, _) = PhysicalPads.deadZone(0.5f, 0f)
        assertTrue(a in 0.4f..0.5f)
        val p = pads()
        p.axes(1, PadAxes(lx = 0.04f, ly = -0.03f))
        p.flush(0)
        assertEquals(0, (sent.last() as Msg.Pad).lx)
    }

    @Test fun `digital triggers from L2 and R2 keys`() {
        val p = pads()
        p.key(1, KeyEvent.KEYCODE_BUTTON_L2, true)
        p.flush(0)
        assertEquals(255, (sent.last() as Msg.Pad).lt)
        assertEquals(0, (sent.last() as Msg.Pad).rt)
    }

    @Test fun `devices take the lowest free id and give it back when unplugged`() {
        val p = pads()
        p.key(10, KeyEvent.KEYCODE_BUTTON_A, true)
        p.key(20, KeyEvent.KEYCODE_BUTTON_B, true)
        p.flush(0)
        assertEquals(1, pad(0).size); assertEquals(1, pad(1).size)
        assertEquals(0, p.idOf(10)); assertEquals(1, p.idOf(20))
        p.removed(10)
        assertEquals(Msg.PadOff(0), sent.last())
        p.key(30, KeyEvent.KEYCODE_BUTTON_Y, true)
        assertEquals("new pad reuses id 0", 0, p.idOf(30))
        assertEquals(1, p.idOf(20))
        assertEquals(20, p.deviceOf(1))
    }

    @Test fun `at most four pads`() {
        val p = pads()
        for (d in 1..6) p.key(d, KeyEvent.KEYCODE_BUTTON_A, true)
        assertEquals(4, p.count)
        assertEquals(null, p.idOf(5))
    }

    @Test fun `first id keeps the on-screen pad's slot free`() {
        val p = pads(firstId = 1)
        for (d in 1..4) p.key(d, KeyEvent.KEYCODE_BUTTON_A, true)
        assertEquals("only ids 1 to 3", 3, p.count)
        p.flush(0)
        assertEquals(0, pad(0).size)
        assertEquals(1, p.idOf(1))
        p.removed(1)
        assertEquals(Msg.PadOff(1), sent.last())
    }

    @Test fun `release all sends pad off for every controller once`() {
        val p = pads()
        p.key(1, KeyEvent.KEYCODE_BUTTON_A, true)
        p.key(2, KeyEvent.KEYCODE_BUTTON_A, true)
        sent.clear()
        p.releaseAll()
        assertEquals(listOf(Msg.PadOff(0), Msg.PadOff(1)), sent)
        sent.clear()
        p.releaseAll()
        assertTrue(sent.isEmpty())
    }

    @Test fun `wire message matches the host format`() {
        val s = PadState(btn = 0x1000, rt = 255, ly = -32768)
        assertEquals(Msg.Pad(2, 0x1000, 0, 255, 0, -32768, 0, 0), s.toMsg(2))
    }
}
