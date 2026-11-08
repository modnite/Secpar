package com.pcremote.settings

import android.content.Context
import android.preference.PreferenceManager
import com.pcremote.agent.KeyValue

enum class ThemeChoice { SYSTEM, DARK, LIGHT }

/** App preferences, all stored as strings so the model is testable over a plain [KeyValue]. */
class SettingsModel(private val kv: KeyValue) {
    private fun float(key: String, def: Float) = kv.get(key)?.toFloatOrNull() ?: def
    private fun bool(key: String, def: Boolean) = kv.get(key)?.toBooleanStrictOrNull() ?: def

    var theme: ThemeChoice
        get() = kv.get(K_THEME)?.let { v -> ThemeChoice.entries.firstOrNull { it.name.equals(v, true) } } ?: ThemeChoice.DARK
        set(v) = kv.put(K_THEME, v.name.lowercase())

    /** Share the text clipboard with the computer. Menu row "Share clipboard"; default on. */
    var shareClipboard: Boolean
        get() = bool(K_SHARE_CLIPBOARD, true)
        set(v) = kv.put(K_SHARE_CLIPBOARD, v.toString())

    /** Play the host's audio. Default on. */
    var playHostAudio: Boolean
        get() = bool(K_PLAY_AUDIO, true)
        set(v) = kv.put(K_PLAY_AUDIO, v.toString())

    /** Stream resolution as width to height; 0 to 0 is the host's own size. Default 1280 x 720. */
    var resolution: Pair<Int, Int>
        get() = kv.get(K_RESOLUTION)?.let(::parseResolution)?.takeIf { it in com.pcremote.stream2.STREAM_SIZES } ?: DEFAULT_RESOLUTION
        set(v) { if (v in com.pcremote.stream2.STREAM_SIZES) kv.put(K_RESOLUTION, "${v.first}x${v.second}") }

    /** Frames per second to ask for. One of [FRAME_RATES]; default 60. */
    var frameRate: Int
        get() = kv.get(K_FPS)?.toIntOrNull()?.takeIf { it in FRAME_RATES } ?: DEFAULT_FPS
        set(v) { if (v in FRAME_RATES) kv.put(K_FPS, v.toString()) }

    /** Bitrate limit in Mbps. One of [BITRATES_MBPS]; default 30. */
    var bitrateMbps: Int
        get() = kv.get(K_BITRATE)?.toIntOrNull()?.takeIf { it in BITRATES_MBPS } ?: DEFAULT_BITRATE_MBPS
        set(v) { if (v in BITRATES_MBPS) kv.put(K_BITRATE, v.toString()) }

    var haptics: Boolean
        get() = bool(K_HAPTICS, true)
        set(v) = kv.put(K_HAPTICS, v.toString())

    var naturalScroll: Boolean
        get() = bool(K_NATURAL, false)
        set(v) = kv.put(K_NATURAL, v.toString())

    var pointerSpeed: Float
        get() = float(K_POINTER, 1f)
        set(v) = kv.put(K_POINTER, v.coerceIn(0.5f, 3f).toString())

    var captureShortcut: com.pcremote.stream2.CaptureShortcut
        get() = kv.get("secpar_capture_shortcut")?.let { value -> com.pcremote.stream2.CaptureShortcut.entries.firstOrNull { it.name == value } }
            ?: com.pcremote.stream2.CaptureShortcut.CTRL_ALT_G
        set(value) = kv.put("secpar_capture_shortcut", value.name)

    var gyroSensitivity: Float
        get() = float(K_GYRO, 1f)
        set(v) = kv.put(K_GYRO, v.coerceIn(0.25f, 3f).toString())

    companion object {
        const val K_THEME = "pcremote_theme"
        const val K_SHARE_CLIPBOARD = "pcremote_share_clipboard"
        const val K_PLAY_AUDIO = "pcremote_play_audio"
        const val K_RESOLUTION = "pcremote_resolution"
        const val K_FPS = "pcremote_frame_rate"
        const val K_BITRATE = "pcremote_bitrate_mbps"
        val DEFAULT_RESOLUTION = 1280 to 720
        val FRAME_RATES = listOf(30, 60, 90, 120)
        const val DEFAULT_FPS = 60
        val BITRATES_MBPS = listOf(5, 10, 20, 30, 50, 80)
        const val DEFAULT_BITRATE_MBPS = 30
        /** A mobile-data link is capped at this bitrate whatever the setting says. */
        const val CELLULAR_CAP_MBPS = 8

        /** Bitrate to request in kbps: the setting, held to the cap on mobile data. */
        fun requestKbps(limitMbps: Int, cellular: Boolean): Int = (if (cellular) minOf(limitMbps, CELLULAR_CAP_MBPS) else limitMbps) * 1000

        internal fun parseResolution(v: String): Pair<Int, Int>? {
            val p = v.split('x')
            if (p.size != 2) return null
            val w = p[0].toIntOrNull() ?: return null
            val h = p[1].toIntOrNull() ?: return null
            return w to h
        }
        const val K_HAPTICS = "pcremote_haptics_s"
        const val K_NATURAL = "pcremote_natural_scroll"
        const val K_POINTER = "pcremote_pointer_speed"
        const val K_GYRO = "pcremote_gyro_sensitivity"

        fun get(ctx: Context) = SettingsModel(prefsKv(ctx))
    }
}

object ScrollMapper {
    fun map(dy: Int, natural: Boolean) = if (natural) -dy else dy
}

/** Default SharedPreferences as a [KeyValue] (string values only). */
fun prefsKv(ctx: Context): KeyValue {
    @Suppress("DEPRECATION")
    val prefs = PreferenceManager.getDefaultSharedPreferences(ctx.applicationContext)
    return object : KeyValue {
        override fun get(key: String) = runCatching { prefs.getString(key, null) }.getOrNull()
        override fun put(key: String, value: String?) { prefs.edit().putString(key, value).apply() }
    }
}
