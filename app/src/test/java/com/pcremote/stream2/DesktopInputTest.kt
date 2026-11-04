package com.pcremote.stream2

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class DesktopInputTest {
    @Test fun mapsPhysicalKeysToWindowsVirtualKeys() {
        assertEquals(0x41, DesktopInput.virtualKey(29))
        assertEquals(0x5A, DesktopInput.virtualKey(54))
        assertEquals(0x70, DesktopInput.virtualKey(131))
        assertEquals(0x7B, DesktopInput.virtualKey(142))
        assertEquals(0xA3, DesktopInput.virtualKey(114))
        assertEquals(0x1B, DesktopInput.virtualKey(111))
        assertEquals(0x60, DesktopInput.virtualKey(144))
        assertNull(DesktopInput.virtualKey(24)) // Volume stays local.
    }
}
