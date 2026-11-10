package com.pcremote.ui.components

import android.os.Build
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.*
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.minimumInteractiveComponentSize
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.Shape
import androidx.annotation.DrawableRes
import androidx.compose.ui.res.painterResource
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

/** Visual button height and minimum width. The touch target stays 48dp around it. */
private val ButtonHeight = 32.dp
private val ButtonMinWidth = 144.dp

@Composable
private fun ButtonFace(text: String, onClick: () -> Unit, modifier: Modifier, enabled: Boolean, @DrawableRes icon: Int?,
    fill: Color, content: Color, border: Color?, compact: Boolean = false) {
    val shape = MidnightShapes.control
    Row(
        modifier.minimumInteractiveComponentSize().defaultMinSize(minWidth = if (compact) 0.dp else ButtonMinWidth, minHeight = ButtonHeight)
            .clip(shape).background(fill).then(if (border != null) Modifier.border(1.dp, border, shape) else Modifier)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .padding(horizontal = 12.dp, vertical = 7.dp),
        horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) { Icon(painterResource(icon), null, tint = content, modifier = Modifier.size(20.dp)); Spacer(Modifier.width(8.dp)) }
        Text(text, color = content, style = MaterialTheme.typography.labelLarge, maxLines = 1)
    }
}

/** The one solid accent button per screen: the thing to do next. 144 x 32, radius 5. */
@Composable
fun PrimaryButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true, @DrawableRes icon: Int? = null,
    compact: Boolean = false) {
    val c = LocalMidnight.current
    val haptics = rememberHaptics()
    ButtonFace(text, { haptics.click(); onClick() }, modifier, enabled, icon,
        fill = if (enabled) c.accent else c.surfaceHigh, content = if (enabled) c.onAccent else c.textDim, border = null, compact = compact)
}

/** Secondary action: surface fill, 1dp stroke. */
@Composable
fun GhostButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, @DrawableRes icon: Int? = null, tint: Color? = null,
    compact: Boolean = false, enabled: Boolean = true) {
    val c = LocalMidnight.current
    val haptics = rememberHaptics()
    ButtonFace(text, { haptics.tick(); onClick() }, modifier, enabled, icon,
        fill = c.surfaceHigh, content = (tint ?: c.text).let { if (enabled) it else c.textDim }, border = c.controlBorder, compact = compact)
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
fun GlassIconButton(@DrawableRes icon: Int, label: String, onClick: () -> Unit, modifier: Modifier = Modifier, tint: Color? = null) {
    val c = LocalMidnight.current
    val haptics = rememberHaptics()
    IconButton(onClick = { haptics.tick(); onClick() }, modifier = modifier.size(48.dp),
        colors = IconButtonDefaults.iconButtonColors(containerColor = c.surface, contentColor = tint ?: c.text)) {
        Icon(painterResource(icon), label, modifier = Modifier.size(20.dp))
    }
}

/** A flat status dot: no pulse, no halo. */
@Composable
fun StatusDot(color: Color, size: Dp = 8.dp) {
    val col by animateColorAsState(color, label = "c")
    Box(Modifier.size(size).clip(CircleShape).background(col))
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


/** Single-line text field: 34dp high, radius 5, 1dp stroke. Same look as the desktop field. */
@Composable
fun FlatField(value: String, onChange: (String) -> Unit, hint: String, modifier: Modifier = Modifier, masked: Boolean = false,
    isError: Boolean = false, mono: Boolean = false,
    keyboardType: androidx.compose.ui.text.input.KeyboardType = androidx.compose.ui.text.input.KeyboardType.Text) {
    val c = LocalMidnight.current
    val shape = MidnightShapes.control
    androidx.compose.foundation.text.BasicTextField(
        value, onChange, modifier.heightIn(min = 34.dp).clip(shape).background(c.bg)
            .border(1.dp, if (isError) c.bad else c.controlBorder, shape),
        singleLine = true,
        keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(keyboardType = keyboardType),
        textStyle = MaterialTheme.typography.bodyMedium.copy(color = c.text,
            fontFamily = if (mono) androidx.compose.ui.text.font.FontFamily.Monospace else MaterialTheme.typography.bodyMedium.fontFamily),
        cursorBrush = androidx.compose.ui.graphics.SolidColor(c.accent),
        visualTransformation = if (masked) androidx.compose.ui.text.input.PasswordVisualTransformation() else androidx.compose.ui.text.input.VisualTransformation.None,
        decorationBox = { inner ->
            Box(Modifier.heightIn(min = 34.dp).padding(horizontal = 10.dp), contentAlignment = Alignment.CenterStart) {
                if (value.isEmpty()) Text(hint, color = c.textDim, style = MaterialTheme.typography.bodyMedium, maxLines = 1)
                inner()
            }
        },
    )
}

/** Checkbox: 20dp square, radius 3, accent fill with a Lucide check, label on the right. 48dp row for touch. */
@Composable
fun FlatCheckbox(checked: Boolean, label: String, onChange: (Boolean) -> Unit, modifier: Modifier = Modifier) {
    val c = LocalMidnight.current
    val haptics = rememberHaptics()
    val box = RoundedCornerShape(3.dp)
    Row(
        modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = Role.Checkbox) { haptics.tick(); onChange(!checked) },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(20.dp).clip(box).background(if (checked) c.accent else c.surfaceHigh)
            .then(if (checked) Modifier else Modifier.border(1.dp, c.controlBorder, box)), contentAlignment = Alignment.Center) {
            if (checked) Icon(painterResource(com.pcremote.R.drawable.lucide_check), null, tint = c.onAccent, modifier = Modifier.size(14.dp))
        }
        Spacer(Modifier.width(10.dp))
        Text(label, color = c.text, style = MaterialTheme.typography.bodyMedium)
    }
}

/** A labelled drop-down: 12 MUTED label above a 32dp control, radius 5. */
@Composable
fun FlatDropdown(label: String, value: String, options: List<String>, onPick: (Int) -> Unit, modifier: Modifier = Modifier) {
    val c = LocalMidnight.current
    var open by remember { mutableStateOf(false) }
    Column(modifier) {
        Text(label, color = c.textDim, style = MaterialTheme.typography.bodySmall)
        Spacer(Modifier.height(4.dp))
        Box {
            Row(
                Modifier.fillMaxWidth().height(32.dp).clip(MidnightShapes.control).background(c.surfaceHigh)
                    .border(1.dp, c.controlBorder, MidnightShapes.control).clickable(role = Role.DropdownList) { open = true }
                    .padding(start = 10.dp, end = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(value, color = c.text, style = MaterialTheme.typography.bodyMedium, maxLines = 1, modifier = Modifier.weight(1f))
                Icon(painterResource(com.pcremote.R.drawable.lucide_chevron_down), null, tint = c.textDim, modifier = Modifier.size(18.dp))
            }
            androidx.compose.material3.DropdownMenu(open, { open = false }, containerColor = c.surfaceHigh,
                shape = MidnightShapes.control, tonalElevation = 0.dp, shadowElevation = 0.dp,
                border = BorderStroke(1.dp, c.controlBorder)) {
                options.forEachIndexed { i, text ->
                    androidx.compose.material3.DropdownMenuItem(
                        text = { Text(text, color = c.text, style = MaterialTheme.typography.bodyMedium) },
                        onClick = { open = false; onPick(i) },
                        trailingIcon = if (text == value) ({ Icon(painterResource(com.pcremote.R.drawable.lucide_check), null, tint = c.accent, modifier = Modifier.size(18.dp)) }) else null,
                    )
                }
            }
        }
    }
}

/** Flat dialog card: surface, radius 7, 1dp border, title 14 strong. */
@Composable
fun FlatDialog(title: String, onDismiss: () -> Unit, actions: @Composable RowScope.() -> Unit, content: @Composable ColumnScope.() -> Unit) {
    val c = LocalMidnight.current
    androidx.compose.ui.window.Dialog(onDismiss) {
        Column(Modifier.fillMaxWidth().widthIn(max = 400.dp).clip(MidnightShapes.card).background(c.surface)
            .border(1.dp, c.border, MidnightShapes.card).padding(16.dp)) {
            Text(title, color = c.text, style = MaterialTheme.typography.titleMedium)
            Spacer(Modifier.height(12.dp))
            content()
            Spacer(Modifier.height(12.dp))
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End),
                verticalAlignment = Alignment.CenterVertically, content = actions)
        }
    }
}

/** Page title: 20 heading, 32dp high, actions right-aligned. */
@Composable
fun PageHeader(title: String, modifier: Modifier = Modifier, actions: @Composable RowScope.() -> Unit = {}) {
    val c = LocalMidnight.current
    Row(modifier.fillMaxWidth().heightIn(min = 32.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(title, color = c.text, style = MaterialTheme.typography.headlineMedium, maxLines = 1,
            overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis, modifier = Modifier.weight(1f).padding(end = 8.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically, content = actions)
    }
}
