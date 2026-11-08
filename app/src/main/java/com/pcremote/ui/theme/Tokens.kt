package com.pcremote.ui.theme

import kotlin.math.max
import kotlin.math.min
import kotlin.math.pow

/** Midnight design tokens as ARGB longs, so they're testable without Compose. */
data class Palette(
    val bg: Long,
    val surface: Long,
    val surfaceHigh: Long,
    val border: Long,
    val accent: Long,
    val accent2: Long,
    val onAccent: Long,
    val ok: Long,
    val warn: Long,
    val bad: Long,
    val text: Long,
    val textDim: Long,
    /** Stroke of secondary buttons and other raised controls (the card border is quieter). */
    val controlBorder: Long = 0xFF414D5E,
)

/** Flat tonal surfaces with the Secpar mint/orange accent palette. */
object Midnight {
    val dark = Palette(
        bg = 0xFF11151C, surface = 0xFF171D27, surfaceHigh = 0xFF1F2733, border = 0xFF313B49,
        accent = 0xFF0AFFB6, accent2 = 0xFFD95400, onAccent = 0xFF07261E,
        ok = 0xFF34D399, warn = 0xFFFBBF24, bad = 0xFFF87171,
        text = 0xFFE6EAF2, textDim = 0xFF98A2B8, controlBorder = 0xFF414D5E,
    )
    val light = Palette(
        bg = 0xFFF3F5FA, surface = 0xFFFFFFFF, surfaceHigh = 0xFFE9EDF5, border = 0x1A05070C,
        accent = 0xFF006B4F, accent2 = 0xFFA94000, onAccent = 0xFFFFFFFF,
        ok = 0xFF0F8A5F, warn = 0xFFA16207, bad = 0xFFC2334A,
        text = 0xFF0B0F1A, textDim = 0xFF566077, controlBorder = 0x3305070C,
    )
}

/**
 * The stream menu floats over the video, so it keeps one look in every theme. Same numbers as the
 * desktop menu (desktop/src/stream_menu.rs). Colours are ARGB longs; sizes are dp.
 */
object MenuTokens {
    const val TEXT = 0xFFE9EBED
    const val MUTED = 0xFF94999E
    const val DANGER = 0xFFF2695E
    const val CHECK = 0xFF0AFFB6
    /** rgba(25,27,29,246): the panel. */
    const val FILL = 0xF6191B1D
    /** rgba(25,27,29,135): the Secpar button. */
    const val BUTTON_FILL = 0x87191B1D
    /** rgba(15,17,19,215): the stats strip. */
    const val STRIP_FILL = 0xD70F1113
    /** White at alpha 12 / 255: pressed or hovered row. */
    const val ROW_PRESSED_ALPHA = 12
    /** White at alpha 23 / 255: dividers. */
    const val DIVIDER_ALPHA = 23

    const val BUTTON = 40
    const val LOGO = 24
    const val BUTTON_MARGIN = 12
    const val PANEL_X = 60
    const val PANEL_Y = 12
    const val PANEL_Y_WITH_STRIP = 34
    const val PANEL_WIDTH = 262
    const val PANEL_PADDING = 6
    const val PANEL_RADIUS = 5
    const val BUTTON_RADIUS = 6
    const val ROW_HEIGHT = 34
    const val ROW_RADIUS = 3
    const val ICON = 20
    const val ICON_X = 9
    const val LABEL_X = 39
    const val TRAILING_ICON = 18
    /** Distance from the row's right edge to the centre of a check or chevron. */
    const val TRAILING_CENTRE = 15
    /** Distance from the row's right edge to the end of a value that sits before a check or chevron. */
    const val VALUE_END_WITH_TRAILING = 30
    /** Distance from the row's right edge to the end of a value with nothing after it. */
    const val VALUE_END = 12
    const val DIVIDER_GAP = 5
    const val FOOTER_SIZE = 10
    const val FOOTER_X = 15
    const val FOOTER_TOP = 5
    const val STRIP_HEIGHT = 28
}

private fun channel(c: Long, shift: Int): Double {
    val v = ((c shr shift) and 0xFF) / 255.0
    return if (v <= 0.03928) v / 12.92 else ((v + 0.055) / 1.055).pow(2.4)
}

private fun luminance(c: Long) = 0.2126 * channel(c, 16) + 0.7152 * channel(c, 8) + 0.0722 * channel(c, 0)

/** WCAG 2 contrast ratio between two opaque ARGB colours. */
fun contrast(a: Long, b: Long): Double {
    val la = luminance(a); val lb = luminance(b)
    return (max(la, lb) + 0.05) / (min(la, lb) + 0.05)
}
