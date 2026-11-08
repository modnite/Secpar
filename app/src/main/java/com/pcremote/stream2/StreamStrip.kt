package com.pcremote.stream2

import android.view.KeyEvent
import java.util.Locale

/** Sizes the stream menu offers; 0x0 is the PC display's own size. */
val STREAM_SIZES: List<Pair<Int, Int>> = listOf(0 to 0, 3840 to 2160, 2560 to 1440, 1920 to 1080, 1280 to 720, 854 to 480)

fun sizeLabel(w: Int, h: Int) = if (w == 0) "Host native" else "$w × $h"

const val RESIZE_UNSUPPORTED = "This computer's Secpar can't resize a live stream yet. Update it there, then pick the resolution again."

/** What the top strip shows when stats are on; parts without a value are left out. */
data class StripStats(
    val w: Int, val h: Int, val fps: Int, val status: String,
    val rttMs: Int, val kbps: Int, val receivedFps: Int, val decoder: String,
)

fun stripText(s: StripStats): String {
    val parts = mutableListOf("${s.w} × ${s.h}", "${s.fps} fps", s.status)
    if (s.rttMs > 0) parts += "RTT ${s.rttMs} ms"
    parts += String.format(Locale.ROOT, "%.1f Mbps", s.kbps / 1000f)
    parts += String.format(Locale.ROOT, "%.1f received fps", s.receivedFps.toFloat())
    if (s.decoder.isNotEmpty()) parts += s.decoder
    return parts.joinToString("   ·   ")
}

/**
 * A size was asked for and the host has not answered with a new `stream.ready` yet. An older host
 * ignores `stream.size`, so after [NOTICE_AFTER_MS] the strip says so, until [GIVE_UP_MS].
 */
class ResizeWait {
    private var askedAt = -1L

    fun asked(nowMs: Long) { askedAt = nowMs }
    /** Any `stream.ready` ends the wait. */
    fun answered() { askedAt = -1L }
    val waiting get() = askedAt >= 0

    fun showNotice(nowMs: Long): Boolean = askedAt >= 0 && nowMs - askedAt in NOTICE_AFTER_MS..GIVE_UP_MS

    /** Delay until the notice appears or goes away, or null when nothing is pending. */
    fun nextChangeMs(nowMs: Long): Long? = if (askedAt < 0) null else when {
        nowMs - askedAt < NOTICE_AFTER_MS -> askedAt + NOTICE_AFTER_MS - nowMs
        nowMs - askedAt <= GIVE_UP_MS -> askedAt + GIVE_UP_MS - nowMs + 1
        else -> { askedAt = -1L; null }
    }

    companion object {
        const val NOTICE_AFTER_MS = 2_500L
        const val GIVE_UP_MS = 8_500L // notice shows about 6 s
    }
}

/** Hardware keyboard chords the stream screen keeps for itself (never sent to the PC). */
enum class StreamChord { DISCONNECT, TOGGLE_BUTTON }

object StreamChords {
    /** Ctrl+Alt+Q disconnects; Ctrl+Alt+S hides or shows the Secpar button. */
    fun match(keyCode: Int, ctrl: Boolean, alt: Boolean): StreamChord? {
        if (!ctrl || !alt) return null
        return when (keyCode) {
            KeyEvent.KEYCODE_Q -> StreamChord.DISCONNECT
            KeyEvent.KEYCODE_S -> StreamChord.TOGGLE_BUTTON
            else -> null
        }
    }
}
