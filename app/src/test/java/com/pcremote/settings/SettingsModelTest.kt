package com.pcremote.settings

import com.pcremote.agent.KeyValue
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SettingsModelTest {
    private class MemKv : KeyValue {
        val map = mutableMapOf<String, String>()
        override fun get(key: String) = map[key]
        override fun put(key: String, value: String?) { if (value == null) map.remove(key) else map[key] = value }
    }

    @Test fun `pointer speed is clamped to half to triple`() {
        val s = SettingsModel(MemKv())
        s.pointerSpeed = 10f; assertEquals(3f, s.pointerSpeed, 0.001f)
        s.pointerSpeed = 0.1f; assertEquals(0.5f, s.pointerSpeed, 0.001f)
        s.pointerSpeed = 1.7f; assertEquals(1.7f, s.pointerSpeed, 0.001f)
    }

    @Test fun `natural scrolling inverts dy`() {
        assertEquals(-120, ScrollMapper.map(120, natural = true))
        assertEquals(120, ScrollMapper.map(120, natural = false))
    }

    @Test fun `theme round-trips and defaults to dark`() {
        val kv = MemKv()
        assertEquals(ThemeChoice.DARK, SettingsModel(kv).theme)
        SettingsModel(kv).theme = ThemeChoice.LIGHT
        assertEquals(ThemeChoice.LIGHT, SettingsModel(kv).theme)
        kv.map["pcremote_theme"] = "garbage"
        assertEquals(ThemeChoice.DARK, SettingsModel(kv).theme)
    }

    @Test fun `haptics default on and persist`() {
        val kv = MemKv()
        assertTrue(SettingsModel(kv).haptics)
        SettingsModel(kv).haptics = false
        assertFalse(SettingsModel(kv).haptics)
    }

    @Test fun `share clipboard and host audio default on and persist`() {
        val kv = MemKv()
        assertTrue(SettingsModel(kv).shareClipboard)
        assertTrue(SettingsModel(kv).playHostAudio)
        SettingsModel(kv).shareClipboard = false
        SettingsModel(kv).playHostAudio = false
        assertFalse(SettingsModel(kv).shareClipboard)
        assertFalse(SettingsModel(kv).playHostAudio)
    }

    @Test fun `resolution defaults to 1280 x 720 and persists a menu size`() {
        val kv = MemKv()
        assertEquals(1280 to 720, SettingsModel(kv).resolution)
        SettingsModel(kv).resolution = 1920 to 1080
        assertEquals(1920 to 1080, SettingsModel(kv).resolution)
        SettingsModel(kv).resolution = 0 to 0
        assertEquals(0 to 0, SettingsModel(kv).resolution)
    }

    @Test fun `a size the menu does not offer is ignored`() {
        val kv = MemKv()
        SettingsModel(kv).resolution = 1234 to 567
        assertEquals(1280 to 720, SettingsModel(kv).resolution)
        kv.map["pcremote_resolution"] = "garbage"
        assertEquals(1280 to 720, SettingsModel(kv).resolution)
        kv.map["pcremote_resolution"] = "999x999"
        assertEquals(1280 to 720, SettingsModel(kv).resolution)
    }

    @Test fun `gyro sensitivity clamped`() {
        val s = SettingsModel(MemKv())
        s.gyroSensitivity = 9f; assertEquals(3f, s.gyroSensitivity, 0.001f)
        s.gyroSensitivity = 0f; assertEquals(0.25f, s.gyroSensitivity, 0.001f)
    }

    @Test fun `frame rate defaults to 60 and keeps only offered values`() {
        val kv = MemKv()
        assertEquals(60, SettingsModel(kv).frameRate)
        SettingsModel(kv).frameRate = 120; assertEquals(120, SettingsModel(kv).frameRate)
        SettingsModel(kv).frameRate = 75; assertEquals(120, SettingsModel(kv).frameRate)
        kv.map["pcremote_frame_rate"] = "garbage"; assertEquals(60, SettingsModel(kv).frameRate)
        assertEquals(listOf(30, 60, 90, 120), SettingsModel.FRAME_RATES)
    }

    @Test fun `bitrate limit defaults to 30 Mbps and keeps only offered values`() {
        val kv = MemKv()
        assertEquals(30, SettingsModel(kv).bitrateMbps)
        SettingsModel(kv).bitrateMbps = 80; assertEquals(80, SettingsModel(kv).bitrateMbps)
        SettingsModel(kv).bitrateMbps = 7; assertEquals(80, SettingsModel(kv).bitrateMbps)
        assertEquals(listOf(5, 10, 20, 30, 50, 80), SettingsModel.BITRATES_MBPS)
    }

    @Test fun `mobile data caps the requested bitrate at 8 Mbps`() {
        assertEquals(30_000, SettingsModel.requestKbps(30, cellular = false))
        assertEquals(8_000, SettingsModel.requestKbps(30, cellular = true))
        assertEquals(5_000, SettingsModel.requestKbps(5, cellular = true))
    }

    @Test fun `dropdown labels match the desktop lists`() {
        assertEquals(listOf("Host native", "3840 × 2160", "2560 × 1440", "1920 × 1080", "1280 × 720", "854 × 480"), SettingsOptions.resolutionLabels)
        assertEquals(listOf("30 fps", "60 fps", "90 fps", "120 fps"), SettingsOptions.frameRateLabels)
        assertEquals(listOf("5 Mbps", "10 Mbps", "20 Mbps", "30 Mbps", "50 Mbps", "80 Mbps"), SettingsOptions.bitrateLabels)
    }
}
