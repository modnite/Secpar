package com.pcremote.stream2

import org.junit.Assert.*
import org.junit.Test

class DesktopCaptureTest {
    @Test fun releaseStaysLocalAcrossRefreshAndUiChanges() {
        val native = mutableListOf<Boolean>()
        var releases = 0
        val capture = DesktopCapture({ releases++ }, { native += it })
        capture.setRequested(true)
        assertTrue(capture.forwarding)
        capture.toggle()
        capture.refresh()
        capture.setUiOpen(true)
        capture.setUiOpen(false)
        assertFalse(capture.forwarding)
        assertFalse(capture.requested)
        assertTrue(releases > 0)
        assertFalse(native.last())
        capture.toggle()
        assertTrue(capture.forwarding)
    }

    @Test fun openingControlsOrLosingFocusRequiresExplicitRecapture() {
        val capture = DesktopCapture({}, {})
        capture.setRequested(true)
        capture.setUiOpen(true)
        capture.setUiOpen(false)
        assertFalse(capture.forwarding)
        capture.setRequested(true)
        capture.lostFocus()
        capture.refresh()
        assertFalse(capture.forwarding)
    }

    @Test fun shortcutDoesNotStealOtherFunctionOrSystemChords() {
        assertTrue(CaptureShortcut.CTRL_F12.matches(142, true, false, false, false))
        assertFalse(CaptureShortcut.CTRL_F12.matches(142, true, false, true, false))
        assertFalse(CaptureShortcut.CTRL_F12.matches(142, true, true, false, false))
        assertFalse(CaptureShortcut.CTRL_F12.matches(142, true, false, false, true))
        assertFalse(CaptureShortcut.CTRL_F12.matches(141, true, false, false, false))
        assertTrue(CaptureShortcut.MOONLIGHT.matches(54, true, true, true, false))
    }
}
