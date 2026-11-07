package com.pcremote.stream

import androidx.compose.animation.*
import androidx.activity.compose.BackHandler
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.foundation.background
import androidx.compose.foundation.Image
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.KeyboardTab
import androidx.compose.material.icons.rounded.*
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.zIndex
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.pcremote.R
import com.pcremote.ui.components.GhostButton
import com.pcremote.ui.components.PrimaryButton
import com.pcremote.ui.components.StatusDot
import com.pcremote.ui.theme.LocalMidnight
import com.pcremote.ui.theme.MidnightShapes
import com.pcremote.ui.theme.rememberHaptics

private const val VK_TAB = 0x09
private const val VK_ESC = 0x1B
private const val VK_LEFT = 0x25
private const val VK_UP = 0x26
private const val VK_RIGHT = 0x27
private const val VK_DOWN = 0x28
private const val VK_DELETE = 0x2E
private const val VK_HOME = 0x24
private const val VK_END = 0x23
private const val VK_PGUP = 0x21
private const val VK_PGDN = 0x22
private const val VK_F1 = 0x70

@Composable
private fun glass() = LocalMidnight.current.surface.copy(alpha = 0.86f)

@Composable
internal fun StreamOverlay(state: ToolbarState, a: ToolbarActions, onLocalUi: (Boolean) -> Unit = {}) {
    var panel by remember { mutableStateOf(false) }
    BackHandler(enabled = panel) { panel = false }
    DisposableEffect(panel, state.keyboardOpen) {
        onLocalUi(panel || state.keyboardOpen)
        onDispose { }
    }
    BoxWithConstraints(Modifier.fillMaxSize().safeDrawingPadding()) {
        AnimatedVisibility(state.connecting != null, enter = fadeIn(), exit = fadeOut()) {
            Connecting(state.pcName, state.connecting.orEmpty(), a::disconnect)
        }
        if (!state.connected) return@BoxWithConstraints
        Box(Modifier.align(Alignment.TopStart).zIndex(2f).padding(12.dp).size(44.dp)
            .clip(androidx.compose.foundation.shape.RoundedCornerShape(6.dp)).background(Color(0xff191b1d).copy(alpha = 0.7f))
            .semantics { contentDescription = "Stream menu" }
            .clickable { panel = !panel }, contentAlignment = Alignment.Center) {
            Image(painterResource(R.drawable.secpar_mark), null, Modifier.size(26.dp))
        }
        if (state.showPerf) StatsPill(state, Modifier.align(Alignment.TopEnd).padding(start = 68.dp, end = 12.dp, top = 12.dp))
        if (state.offerLowerQuality) LowerQualityChip(a, Modifier.align(Alignment.TopCenter).padding(top = 12.dp))
        AnimatedVisibility(state.keyboardOpen, Modifier.align(Alignment.TopCenter).padding(top = 56.dp),
            enter = slideInVertically { -it } + fadeIn(), exit = slideOutVertically { -it } + fadeOut()) {
            KeyboardBar(state, a)
        }
        // Scrim + side panel
        AnimatedVisibility(panel, enter = fadeIn(), exit = fadeOut()) {
            Box(Modifier.fillMaxSize().background(Color.Transparent)
                .clickable(remember { MutableInteractionSource() }, null) { panel = false })
        }
        AnimatedVisibility(panel, Modifier.align(Alignment.TopStart).padding(start = if (maxWidth >= 320.dp) 68.dp else 12.dp, top = if (maxWidth >= 320.dp) 12.dp else 64.dp, end = 12.dp, bottom = 12.dp),
            enter = fadeIn(), exit = fadeOut()) {
            EdgePanel(state, a, Modifier.heightIn(max = (maxHeight - if (maxWidth >= 320.dp) 24.dp else 76.dp).coerceAtLeast(120.dp))) { panel = false }
        }

    }
}

@Composable
private fun StatsPill(state: ToolbarState, modifier: Modifier) {
    Text(state.perfText.ifEmpty { "Waiting for stream stats" }, color = Color(0xffe9ebed),
        fontSize = 11.sp, lineHeight = 15.sp, fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace,
        modifier = modifier.widthIn(max = 400.dp)
            .clip(androidx.compose.foundation.shape.RoundedCornerShape(4.dp))
            .background(Color(0xff101214).copy(alpha = 0.88f)).clickable { state.showPerf = false }.padding(9.dp))
}

@Composable
private fun LowerQualityChip(a: ToolbarActions, modifier: Modifier) {
    val c = LocalMidnight.current
    Row(
        modifier.clip(MidnightShapes.pill).background(glass()).border(1.dp, c.warn.copy(alpha = 0.6f), MidnightShapes.pill)
            .clickable(onClick = a::lowerQuality).padding(horizontal = 14.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Rounded.NetworkCheck, null, tint = c.warn, modifier = Modifier.size(18.dp))
        Spacer(Modifier.width(8.dp))
        Text(stringResource(R.string.pcr_tb_lower_quality), color = c.text, style = MaterialTheme.typography.labelLarge)
    }
}

@Composable
private fun EdgePanel(state: ToolbarState, a: ToolbarActions, modifier: Modifier, close: () -> Unit) {
    var page by remember { mutableIntStateOf(0) }
    val context = LocalContext.current
    Column(modifier.widthIn(max = 268.dp).fillMaxWidth()
        .clip(androidx.compose.foundation.shape.RoundedCornerShape(5.dp))
        .background(Color(0xff191b1d).copy(alpha = 0.97f))
        .verticalScroll(rememberScrollState()).padding(vertical = 6.dp)) {
        if (page == 0) {
            StreamMenuRow(R.drawable.lucide_keyboard, "Keyboard", checked = state.keyboardOpen) { a.keyboard(); close() }
            StreamMenuRow(R.drawable.lucide_mouse, "Trackpad", checked = state.trackpad) {
                state.trackpad = !state.trackpad; a.setTrackpad(state.trackpad); close()
            }
            if (state.canPip) StreamMenuRow(R.drawable.lucide_picture_in_picture_2, "Picture in picture") { close(); a.pictureInPicture() }
            if (state.displays.size > 1) StreamMenuRow(R.drawable.lucide_monitor, "Switch display", arrow = true) { page = 3 }
            StreamMenuRow(R.drawable.lucide_chart_no_axes_column_increasing, "Stream stats", checked = state.showPerf) { state.showPerf = !state.showPerf; close() }
            StreamMenuRow(R.drawable.lucide_command, "More keys", arrow = true) { page = 1 }
            StreamMenuRow(R.drawable.lucide_circle_help, "Help") { context.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW,
                android.net.Uri.parse("https://secpar.win/help/#stream-menu"))) }
            androidx.compose.material3.HorizontalDivider(Modifier.padding(horizontal = 10.dp, vertical = 4.dp), color = Color.White.copy(alpha = 0.09f))
            StreamMenuRow(R.drawable.lucide_log_out, "Disconnect", destructive = true) { a.disconnect() }
            Text("Secpar " + com.pcremote.BuildConfig.VERSION_NAME, color = Color(0xff94999e), fontSize = 10.sp,
                modifier = Modifier.padding(start = 14.dp, top = 4.dp, bottom = 2.dp))
        } else {
            StreamMenuRow(R.drawable.lucide_arrow_left, when(page) { 1 -> "Keys"; 2 -> "Media keys"; else -> "Displays" }) { page = if (page == 2) 1 else 0 }
            androidx.compose.material3.HorizontalDivider(Modifier.padding(horizontal = 10.dp, vertical = 4.dp), color = Color.White.copy(alpha = 0.09f))
            when (page) {
                1 -> {
                    StreamMenuRow(R.drawable.lucide_corner_down_left, "Escape") { a.keys(chord(listOf(VK_ESC))); close() }
                    StreamMenuRow(R.drawable.lucide_panels_top_left, "Windows key") { a.shortcut(Shortcut.WIN); close() }
                    StreamMenuRow(R.drawable.lucide_arrow_left_right, "Switch app", value = "Alt+Tab") { a.shortcut(Shortcut.ALT_TAB); close() }
                    StreamMenuRow(R.drawable.lucide_copy, "Copy", value = "Ctrl+C") { a.shortcut(Shortcut.COPY); close() }
                    StreamMenuRow(R.drawable.lucide_clipboard_paste, "Paste", value = "Ctrl+V") { a.shortcut(Shortcut.PASTE); close() }
                    StreamMenuRow(R.drawable.lucide_panels_top_left, "Show desktop") { a.keys(chord(listOf(0x5B, 0x44))); close() }
                    StreamMenuRow(R.drawable.lucide_monitor, "Task Manager") { a.keys(chord(listOf(0x11, 0x10, VK_ESC))); close() }
                    StreamMenuRow(R.drawable.lucide_x, "Close window", value = "Alt+F4") { a.keys(chord(listOf(0x12, 0x73))); close() }
                    StreamMenuRow(R.drawable.lucide_music_2, "Media keys", arrow = true) { page = 2 }
                }
                2 -> {
                    for ((icon, label, shortcut) in listOf(
                        Triple(R.drawable.lucide_skip_back,"Previous",Shortcut.PREV),
                        Triple(R.drawable.lucide_play,"Play / pause",Shortcut.PLAY_PAUSE),
                        Triple(R.drawable.lucide_skip_forward,"Next",Shortcut.NEXT),
                        Triple(R.drawable.lucide_volume_1,"Host volume down",Shortcut.VOL_DOWN),
                        Triple(R.drawable.lucide_volume_2,"Host volume up",Shortcut.VOL_UP),
                        Triple(R.drawable.lucide_volume_x,"Mute host",Shortcut.MUTE))) {
                        StreamMenuRow(icon, label) { a.shortcut(shortcut) }
                    }
                }
                3 -> state.displays.forEach { display -> StreamMenuRow(R.drawable.lucide_monitor, display.name, checked = display.id == state.display) {
                    state.display = display.id; a.selectDisplay(display.id); close()
                } }
            }
        }
    }
}

@Composable
private fun StreamMenuRow(icon: Int, label: String, value: String? = null, arrow: Boolean = false,
    checked: Boolean = false, destructive: Boolean = false, onClick: () -> Unit) {
    val color = if (destructive) Color(0xfff2695e) else Color(0xffe9ebed)
    Row(Modifier.fillMaxWidth().height(42.dp).semantics { contentDescription = label }
        .clickable(onClick = onClick).padding(horizontal = 14.dp), verticalAlignment = Alignment.CenterVertically) {
        Icon(painterResource(icon), null, tint = color, modifier = Modifier.size(21.dp))
        Spacer(Modifier.width(14.dp))
        Text(label, color = color, fontSize = 14.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
        value?.let { Text(it, color = Color(0xff94999e), fontSize = 12.sp, maxLines = 1) }
        if (checked) Icon(painterResource(R.drawable.lucide_check), null, tint = Color(0xff0affb6), modifier = Modifier.size(18.dp))
        else if (arrow) Icon(painterResource(R.drawable.lucide_chevron_right), null, tint = Color(0xff94999e), modifier = Modifier.size(18.dp))
    }
}

@Composable
private fun KeyboardBar(state: ToolbarState, a: ToolbarActions) {
    val c = LocalMidnight.current
    val haptics = rememberHaptics()
    var fn by remember { mutableStateOf(false) }
    @Suppress("UNUSED_EXPRESSION") state.barTick // recompose when modifiers change

    @Composable
    fun Cap(text: String? = null, icon: ImageVector? = null, mod: Mod? = null, onClick: () -> Unit) {
        val st = mod?.let { state.bar.state(it) } ?: ModState.OFF
        val bg = when (st) { ModState.LOCKED -> c.accent; ModState.ONE_SHOT -> c.accent.copy(alpha = 0.3f); else -> c.surfaceHigh }
        val fg = if (st == ModState.LOCKED) c.onAccent else c.text
        Box(
            Modifier.height(40.dp).widthIn(min = 44.dp).clip(MidnightShapes.control).background(bg)
                .clickable { haptics.tick(); onClick() }.padding(horizontal = 12.dp),
            contentAlignment = Alignment.Center,
        ) {
            if (icon != null) Icon(icon, text, tint = fg, modifier = Modifier.size(20.dp))
            else Text(text ?: "", color = fg, style = MaterialTheme.typography.labelLarge, fontWeight = FontWeight.SemiBold)
        }
    }

    fun press(vk: Int) = a.keys(state.bar.key(vk)).also { state.barTick++ }
    fun tapMod(m: Mod) { state.bar.tap(m, System.currentTimeMillis()); state.barTick++ }

    Row(
        Modifier.clip(MidnightShapes.card).background(glass()).border(1.dp, c.border, MidnightShapes.card)
            .horizontalScroll(rememberScrollState()).padding(8.dp),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        if (fn) {
            for (i in 0 until 12) Cap("F${i + 1}") { press(VK_F1 + i) }
            Cap(icon = Icons.Rounded.KeyboardReturn) { fn = false }
        } else {
            Cap("Esc") { press(VK_ESC) }
            Cap(icon = Icons.AutoMirrored.Rounded.KeyboardTab) { press(VK_TAB) }
            Cap("Ctrl", mod = Mod.CTRL) { tapMod(Mod.CTRL) }
            Cap("Alt", mod = Mod.ALT) { tapMod(Mod.ALT) }
            Cap("Shift", mod = Mod.SHIFT) { tapMod(Mod.SHIFT) }
            Cap(icon = Icons.Rounded.Window, mod = Mod.WIN) { tapMod(Mod.WIN) }
            Cap(icon = Icons.Rounded.KeyboardArrowLeft) { press(VK_LEFT) }
            Cap(icon = Icons.Rounded.KeyboardArrowUp) { press(VK_UP) }
            Cap(icon = Icons.Rounded.KeyboardArrowDown) { press(VK_DOWN) }
            Cap(icon = Icons.Rounded.KeyboardArrowRight) { press(VK_RIGHT) }
            Cap("Fn") { fn = true }
            Cap("Del") { press(VK_DELETE) }
            Cap("Home") { press(VK_HOME) }
            Cap("End") { press(VK_END) }
            Cap("PgUp") { press(VK_PGUP) }
            Cap("PgDn") { press(VK_PGDN) }
        }
    }
}



/** Midnight connecting screen (replaces Moonlight's spinner dialog). */
@Composable
private fun Connecting(pcName: String?, stage: String, cancel: () -> Unit) {
    val c = LocalMidnight.current
    Box(Modifier.fillMaxSize().background(c.bg), contentAlignment = Alignment.Center) {
        com.pcremote.ui.components.Aurora(intensity = 0.7f)
        Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.padding(32.dp)) {
            androidx.compose.material3.CircularProgressIndicator(color = c.accent, trackColor = c.surfaceHigh, strokeWidth = 3.dp,
                modifier = Modifier.size(48.dp))
            Spacer(Modifier.height(28.dp))
            Text(stringResource(R.string.pcr_connecting_to, pcName ?: "PC"), color = c.text, style = MaterialTheme.typography.headlineSmall)
            Spacer(Modifier.height(8.dp))
            AnimatedContent(stage, transitionSpec = { fadeIn() togetherWith fadeOut() }, label = "stage") {
                Text(it, color = c.textDim, style = MaterialTheme.typography.bodyMedium)
            }
            Spacer(Modifier.height(32.dp))
            GhostButton(stringResource(R.string.pcr_cancel), cancel)
        }
    }
}
