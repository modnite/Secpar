package com.pcremote.stream2

import android.view.KeyEvent
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class StreamStripTest {
    @Test fun `sizes are the six menu items with native first`() {
        assertEquals(listOf(0 to 0, 3840 to 2160, 2560 to 1440, 1920 to 1080, 1280 to 720, 854 to 480), STREAM_SIZES)
        assertEquals("Host native", sizeLabel(0, 0))
        assertEquals("1280 × 720", sizeLabel(1280, 720))
    }

    @Test fun `strip joins the parts with spaced dots`() {
        val s = StripStats(1920, 1080, 60, "Direct", 12, 24_500, 59, "c2.qti.hevc.decoder")
        assertEquals("1920 × 1080   ·   60 fps   ·   Direct   ·   RTT 12 ms   ·   24.5 Mbps   ·   59.0 received fps   ·   c2.qti.hevc.decoder",
            stripText(s))
    }

    @Test fun `strip leaves out what is unknown`() {
        assertEquals("1280 × 720   ·   30 fps   ·   Connecting   ·   0.0 Mbps   ·   0.0 received fps",
            stripText(StripStats(1280, 720, 30, "Connecting", 0, 0, 0, "")))
    }

    @Test fun `notice shows from 2_5 s for about 6 s and an answer cancels it`() {
        val w = ResizeWait()
        assertFalse(w.showNotice(0)); assertNull(w.nextChangeMs(0))
        w.asked(1_000)
        assertTrue(w.waiting)
        assertFalse(w.showNotice(3_499))
        assertEquals(1_500L, w.nextChangeMs(2_000))
        assertTrue(w.showNotice(3_500))
        assertTrue(w.showNotice(9_500))
        assertFalse(w.showNotice(9_501))
        assertEquals(1L, w.nextChangeMs(9_500))
        assertNull(w.nextChangeMs(20_000))
        assertFalse(w.waiting)
    }

    @Test fun `a stream ready inside 2_5 s means no notice`() {
        val w = ResizeWait()
        w.asked(0); w.answered()
        assertFalse(w.showNotice(5_000)); assertFalse(w.waiting)
    }

    @Test fun `a second pick restarts the timer`() {
        val w = ResizeWait()
        w.asked(0); w.asked(2_000)
        assertFalse(w.showNotice(4_000)); assertTrue(w.showNotice(4_500))
    }

    @Test fun `chords need Ctrl and Alt`() {
        assertEquals(StreamChord.DISCONNECT, StreamChords.match(KeyEvent.KEYCODE_Q, ctrl = true, alt = true))
        assertEquals(StreamChord.TOGGLE_BUTTON, StreamChords.match(KeyEvent.KEYCODE_S, ctrl = true, alt = true))
        assertNull(StreamChords.match(KeyEvent.KEYCODE_Q, ctrl = true, alt = false))
        assertNull(StreamChords.match(KeyEvent.KEYCODE_Q, ctrl = false, alt = true))
        assertNull(StreamChords.match(KeyEvent.KEYCODE_W, ctrl = true, alt = true))
    }
}
