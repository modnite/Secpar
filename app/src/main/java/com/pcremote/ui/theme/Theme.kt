package com.pcremote.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** Palette as Compose colours, read via [LocalMidnight]. */
@Immutable
data class MidnightColors(
    val bg: Color, val surface: Color, val surfaceHigh: Color, val border: Color,
    val accent: Color, val accent2: Color, val onAccent: Color,
    val ok: Color, val warn: Color, val bad: Color, val text: Color, val textDim: Color,
    val controlBorder: Color,
    val isDark: Boolean,
)

private fun Palette.toColors(dark: Boolean) = MidnightColors(
    Color(bg), Color(surface), Color(surfaceHigh), Color(border),
    Color(accent), Color(accent2), Color(onAccent),
    Color(ok), Color(warn), Color(bad), Color(text), Color(textDim), Color(controlBorder), dark,
)

val LocalMidnight = staticCompositionLocalOf { Midnight.dark.toColors(true) }

object MidnightShapes {
    /** Cards and settings groups. */
    val card = RoundedCornerShape(7.dp)
    val control = RoundedCornerShape(5.dp)
    val small = RoundedCornerShape(5.dp)
    /** Only the status chip is a pill. */
    val pill = RoundedCornerShape(50)
}

/** Stream menu colours (fixed in every theme: the menu sits over the video). */
object MenuColors {
    val text = Color(MenuTokens.TEXT)
    val muted = Color(MenuTokens.MUTED)
    val danger = Color(MenuTokens.DANGER)
    val check = Color(MenuTokens.CHECK)
    val fill = Color(MenuTokens.FILL)
    val buttonFill = Color(MenuTokens.BUTTON_FILL)
    val stripFill = Color(MenuTokens.STRIP_FILL)
    val rowPressed = Color.White.copy(alpha = MenuTokens.ROW_PRESSED_ALPHA / 255f)
    val divider = Color.White.copy(alpha = MenuTokens.DIVIDER_ALPHA / 255f)
}

/** Sizes in sp. Body 14/20, page heading 20, group and card titles 14 strong, small 12. */
object TypeScale {
    const val BODY = 14
    const val BODY_LINE = 20
    const val HEADING = 20
    const val HEADING_LINE = 28
    const val TITLE = 14
    const val SMALL = 12
    const val SMALL_LINE = 16
}

private val sans = androidx.compose.ui.text.font.FontFamily.SansSerif
private fun style(size: Int, line: Int, weight: FontWeight) = TextStyle(
    fontFamily = sans, fontWeight = weight, fontSize = size.sp, lineHeight = line.sp, letterSpacing = 0.sp,
)

private val type = Typography().run {
    val body = style(TypeScale.BODY, TypeScale.BODY_LINE, FontWeight.Normal)
    val heading = style(TypeScale.HEADING, TypeScale.HEADING_LINE, FontWeight.SemiBold)
    val title = style(TypeScale.TITLE, TypeScale.BODY_LINE, FontWeight.SemiBold)
    val small = style(TypeScale.SMALL, TypeScale.SMALL_LINE, FontWeight.Normal)
    copy(
        headlineLarge = heading, headlineMedium = heading, headlineSmall = heading,
        titleLarge = title, titleMedium = title, titleSmall = title,
        bodyLarge = body, bodyMedium = body, bodySmall = small,
        labelLarge = style(TypeScale.BODY, TypeScale.BODY_LINE, FontWeight.Medium),
        labelMedium = small,
    )
}

/** Big tabular numbers for latency, PIN, timers. */
val NumberStyle = TextStyle(fontSize = 44.sp, fontWeight = FontWeight.SemiBold, letterSpacing = (-1).sp,
    fontFeatureSettings = "tnum")

@Composable
fun MidnightTheme(
    dark: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    val base = (if (dark) Midnight.dark else Midnight.light).toColors(dark)
    val c = base
    val scheme = (if (dark) darkColorScheme() else lightColorScheme()).copy(
        primary = c.accent, onPrimary = c.onAccent, secondary = c.accent2, tertiary = c.accent2,
        background = c.bg, onBackground = c.text, surface = c.surface, onSurface = c.text,
        surfaceVariant = c.surfaceHigh, onSurfaceVariant = c.textDim, surfaceContainer = c.surface,
        surfaceContainerHigh = c.surfaceHigh, surfaceContainerHighest = c.surfaceHigh,
        surfaceContainerLow = c.surface, surfaceContainerLowest = c.bg,
        outline = c.textDim, outlineVariant = c.border, error = c.bad,
    )
    CompositionLocalProvider(LocalMidnight provides c) {
        MaterialTheme(colorScheme = scheme, typography = type, content = content)
    }
}
