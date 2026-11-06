package com.pcremote.modes

import com.pcremote.agent.Msg
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ClipSyncTest {
    @Test fun `first focus seeds without sending preexisting text`() {
        val s = ClipSync()
        assertNull(s.onFocus("old android text"))
        assertNull(s.onFocus("old android text"))
    }

    @Test fun `copy in another app is sent on return once`() {
        val s = ClipSync()
        s.onFocus("initial")
        assertEquals(Msg.Clip("text/plain", "copied elsewhere"), s.onFocus("copied elsewhere"))
        assertNull(s.onFocus("copied elsewhere"))
    }

    @Test fun `initial empty clipboard still seeds the first focus`() {
        val s = ClipSync()
        assertNull(s.onFocus(null))
        assertEquals(Msg.Clip("text/plain", "new"), s.onFocus("new"))
    }

    @Test fun `focused local copy is sent to the PC once`() {
        val s = ClipSync()
        s.onFocus("initial")
        assertEquals(Msg.Clip("text/plain", "hello"), s.onLocalChange("hello"))
        assertNull(s.onLocalChange("hello"))
        assertNull(s.onFocus("hello"))
    }

    @Test fun `remote clipboard does not echo through listener or returning focus`() {
        val s = ClipSync()
        s.onFocus("initial")
        assertEquals("from pc", s.onRemote(Msg.Clip("text/plain", "from pc")))
        assertNull(s.onLocalChange("from pc"))
        assertNull(s.onFocus("from pc"))
        assertNull(s.onRemote(Msg.Clip("text/plain", "from pc")))
        assertEquals(Msg.Clip("text/plain", "new"), s.onFocus("new"))
    }

    @Test fun `remote text received before first focus is not echoed`() {
        val s = ClipSync()
        s.onRemote(Msg.Clip("text/plain", "from pc"))
        assertNull(s.onFocus("from pc"))
        assertNull(s.onLocalChange("from pc"))
    }

    @Test fun `reconnect or new session gets a fresh baseline and no old echo state`() {
        val previous = ClipSync()
        previous.onFocus("initial")
        previous.onRemote(Msg.Clip("text/plain", "previous pc"))
        // The composable creates a fresh policy on session replacement or online transition.
        val reconnected = ClipSync()
        assertNull(reconnected.onFocus("copied while disconnected"))
        assertEquals("previous pc", reconnected.onRemote(Msg.Clip("text/plain", "previous pc")))
        val otherSession = ClipSync()
        assertNull(otherSession.onFocus("previous pc"))
        assertEquals(Msg.Clip("text/plain", "initial"), otherSession.onFocus("initial"))
    }

    @Test fun `size bound uses UTF8 bytes on both directions`() {
        val s = ClipSync()
        s.onFocus(null)
        val boundary = "é".repeat(ClipSync.MAX_TEXT / 2)
        val oversized = boundary + "a"
        assertNull(s.onLocalChange(oversized))
        assertNull(s.onRemote(Msg.Clip("text/plain", oversized)))
        assertEquals(Msg.Clip("text/plain", boundary), s.onLocalChange(boundary))
        assertNull(s.onFocus(boundary))
        assertEquals(boundary, ClipSync().onRemote(Msg.Clip("text/plain", boundary)))
        assertNull(s.onLocalChange("a".repeat(ClipSync.MAX_TEXT + 1)))
    }

    @Test fun `oversized first baseline never pushes text and does not block a later valid copy`() {
        val s = ClipSync()
        assertNull(s.onFocus("a".repeat(ClipSync.MAX_TEXT + 1)))
        assertEquals(Msg.Clip("text/plain", "valid"), s.onFocus("valid"))
    }

    @Test fun `images from the PC are ignored on Android`() {
        assertNull(ClipSync().onRemote(Msg.Clip("image/png", "iVBOR")))
    }
}
