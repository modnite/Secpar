package com.pcremote.ui.components

import android.os.Build
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.*
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.pcremote.ui.theme.LocalMidnight
import com.pcremote.ui.theme.MidnightShapes
import com.pcremote.ui.theme.Springs
import com.pcremote.ui.theme.rememberHaptics

/** Plain backgrounds and tonal surfaces shared by app screens. */
@Composable
fun Aurora(modifier: Modifier = Modifier, intensity: Float = 1f, screen: ImageBitmap? = null) {
    Box(modifier.fillMaxSize().background(LocalMidnight.current.bg))
}

fun Modifier.glass(shape: Shape = MidnightShapes.card, strong: Boolean = false): Modifier = composed {
    val c = LocalMidnight.current
    clip(shape).background(if (strong) c.surfaceHigh else c.surface)
}

/** The one solid accent button per screen: the thing to do next. */
@Composable
fun PrimaryButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true, icon: ImageVector? = null) {
    val c = LocalMidnight.current
    val haptics = rememberHaptics()
    val src = remember { MutableInteractionSource() }
    val pressed by src.collectIsPressedAsState()
    val scale by animateFloatAsState(if (pressed) 0.96f else 1f, Springs.bouncy(), label = "s")
    val bg by animateColorAsState(if (enabled) c.accent else c.surfaceHigh, label = "bg")
    Row(
        modifier
            .scale(scale)
            .clip(MidnightShapes.pill)
            .background(bg)
            .clickable(src, null, enabled, role = Role.Button) { haptics.click(); onClick() }
            .padding(horizontal = 24.dp, vertical = 15.dp),
        horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) { Icon(icon, null, tint = if (enabled) c.onAccent else c.textDim, modifier = Modifier.size(20.dp)); Spacer(Modifier.width(10.dp)) }
        Text(text, color = if (enabled) c.onAccent else c.textDim, style = MaterialTheme.typography.labelLarge)
    }
}

/** A quiet glass pill for secondary actions. */
@Composable
fun GhostButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, icon: ImageVector? = null, tint: Color? = null) {
    val c = LocalMidnight.current
    val haptics = rememberHaptics()
    Row(
        modifier.glass(MidnightShapes.pill).clickable(role = Role.Button) { haptics.tick(); onClick() }.padding(horizontal = 18.dp, vertical = 12.dp),
        horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) { Icon(icon, null, tint = tint ?: c.text, modifier = Modifier.size(18.dp)); Spacer(Modifier.width(8.dp)) }
        Text(text, color = tint ?: c.text, style = MaterialTheme.typography.labelLarge)
    }
}

/** A glass panel. */
@Composable
fun GlassCard(modifier: Modifier = Modifier, onClick: (() -> Unit)? = null, padding: Dp = 20.dp, content: @Composable ColumnScope.() -> Unit) {
    val haptics = rememberHaptics()
    Column(
        modifier.glass().then(if (onClick != null) Modifier.clickable { haptics.tick(); onClick() } else Modifier).padding(padding),
        content = content,
    )
}

/** A round glass icon button. */
@Composable
fun GlassIconButton(icon: ImageVector, label: String, onClick: () -> Unit, modifier: Modifier = Modifier, tint: Color? = null) {
    val c = LocalMidnight.current
    val haptics = rememberHaptics()
    Box(
        modifier.size(44.dp).glass(CircleShape).clickable(role = Role.Button, onClickLabel = label) { haptics.tick(); onClick() },
        contentAlignment = Alignment.Center,
    ) { Icon(icon, label, tint = tint ?: c.text, modifier = Modifier.size(20.dp)) }
}

/** A status dot that breathes while [live]. */
@Composable
fun StatusDot(color: Color, live: Boolean, size: Dp = 10.dp) {
    val pulse by rememberInfiniteTransition(label = "dot").animateFloat(
        1f, 2.4f, infiniteRepeatable(tween(1800, easing = FastOutSlowInEasing)), label = "p",
    )
    val col by animateColorAsState(color, label = "c")
    Box(Modifier.size(size * 2.2f), contentAlignment = Alignment.Center) {
        if (live) Box(Modifier.size(size).scale(pulse).clip(CircleShape).background(col.copy(alpha = (2.4f - pulse) / 3.5f)))
        Box(Modifier.size(size).clip(CircleShape).background(col))
    }
}

/** A small header for a group of settings or items. */
@Composable
fun SectionLabel(text: String, modifier: Modifier = Modifier) {
    val c = LocalMidnight.current
    Text(text, color = c.textDim, style = MaterialTheme.typography.titleSmall, modifier = modifier.padding(start = 6.dp, top = 12.dp, bottom = 4.dp))
}

/** Animated page indicator: the active dot stretches into a pill. */
@Composable
fun PagerDots(count: Int, current: Int) {
    val c = LocalMidnight.current
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        repeat(count) { i ->
            val w by animateDpAsState(if (i == current) 24.dp else 8.dp, Springs.snappy(), label = "w")
            Box(Modifier.height(8.dp).width(w).clip(CircleShape).background(if (i == current) c.accent else c.surfaceHigh))
        }
    }
}
