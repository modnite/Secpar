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
import androidx.compose.foundation.hoverable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
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
import com.pcremote.stream2.RESIZE_UNSUPPORTED
import com.pcremote.stream2.STREAM_SIZES
import com.pcremote.stream2.sizeLabel
import com.pcremote.ui.components.GhostButton
import com.pcremote.ui.components.PrimaryButton
import com.pcremote.ui.components.StatusDot
import com.pcremote.ui.theme.LocalMidnight
import com.pcremote.ui.theme.MenuColors
import com.pcremote.ui.theme.MenuTokens
import kotlinx.coroutines.delay
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
        LaunchedEffect(state.buttonHidden) { if (state.buttonHidden) panel = false }
        if (!state.buttonHidden) Box(Modifier.align(Alignment.TopStart).zIndex(2f).padding(MenuTokens.BUTTON_MARGIN.dp).size(MenuTokens.BUTTON.dp)
            .clip(RoundedCornerShape(MenuTokens.BUTTON_RADIUS.dp)).background(MenuColors.buttonFill)
            .semantics { contentDescription = "Stream menu" }
            .clickable { panel = !panel }, contentAlignment = Alignment.Center) {
            Image(painterResource(R.drawable.secpar_mark), null, Modifier.size(MenuTokens.LOGO.dp))
        }
        val strip = state.showPerf || state.resizeNotice
        if (strip) StatsStrip(StreamMenuModel.stripText(state.resizeNotice, RESIZE_UNSUPPORTED, state.perfText),
            if (state.buttonHidden) 12.dp else 60.dp, Modifier.align(Alignment.TopStart))
        AnimatedVisibility(state.keyboardOpen, Modifier.align(Alignment.TopCenter).padding(top = 56.dp),
            enter = slideInVertically { -it } + fadeIn(), exit = slideOutVertically { -it } + fadeOut()) {
            KeyboardBar(state, a)
        }
        // Scrim + side panel
        AnimatedVisibility(panel, enter = fadeIn(), exit = fadeOut()) {
            Box(Modifier.fillMaxSize().background(Color.Transparent)
                .clickable(remember { MutableInteractionSource() }, null) { panel = false })
        }
        val wide = maxWidth >= 320.dp
        val top = if (wide) StreamMenuModel.panelTop(strip).dp else 64.dp
        AnimatedVisibility(panel, Modifier.align(Alignment.TopStart).padding(start = if (wide) MenuTokens.PANEL_X.dp else 12.dp, top = top, end = 12.dp, bottom = 12.dp),
            enter = fadeIn(), exit = fadeOut()) {
            EdgePanel(state, a, Modifier.heightIn(max = (maxHeight - top - 12.dp).coerceAtLeast(120.dp))) { panel = false }
        }

    }
}

@Composable
private fun StatsStrip(text: String, start: androidx.compose.ui.unit.Dp, modifier: Modifier) {
    Box(modifier.fillMaxWidth().height(MenuTokens.STRIP_HEIGHT.dp).background(MenuColors.stripFill).padding(horizontal = start),
        contentAlignment = Alignment.Center) {
        Text(text, color = MenuColors.text, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
    }
}

@Composable
internal fun MenuDivider() {
    Box(Modifier.fillMaxWidth().padding(vertical = MenuTokens.DIVIDER_GAP.dp).height(1.dp).background(MenuColors.divider))
}

@Composable
private fun EdgePanel(state: ToolbarState, a: ToolbarActions, modifier: Modifier, close: () -> Unit) {
    var page by remember { mutableStateOf(MenuPage.MAIN) }
    var audioDetails by remember { mutableStateOf("") }
    val context = LocalContext.current
    if (page == MenuPage.AUDIO) LaunchedEffect(Unit) {
        while (true) { audioDetails = a.audioDetails(); delay(1000) }
    }
    Column(modifier.widthIn(max = MenuTokens.PANEL_WIDTH.dp).fillMaxWidth()
        .clip(RoundedCornerShape(MenuTokens.PANEL_RADIUS.dp))
        .background(MenuColors.fill)
        .verticalScroll(rememberScrollState()).padding(MenuTokens.PANEL_PADDING.dp)) {
        if (page == MenuPage.MAIN) {
            for (row in StreamMenuModel.mainRows(state.canPip, state.displays.size)) when (row) {
                MenuRow.KEYBOARD -> StreamMenuRow(R.drawable.lucide_keyboard, row.label, checked = state.keyboardOpen) { a.keyboard(); close() }
                MenuRow.TRACKPAD -> StreamMenuRow(R.drawable.lucide_mouse, row.label, checked = state.trackpad) {
                    state.trackpad = !state.trackpad; a.setTrackpad(state.trackpad); close()
                }
                MenuRow.PICTURE_IN_PICTURE -> StreamMenuRow(R.drawable.lucide_picture_in_picture_2, row.label) { close(); a.pictureInPicture() }
                MenuRow.RESOLUTION -> StreamMenuRow(R.drawable.lucide_monitor, row.label, value = sizeLabel(state.sizeW, state.sizeH), arrow = true) { page = MenuPage.RESOLUTION }
                MenuRow.SWITCH_DISPLAY -> StreamMenuRow(R.drawable.lucide_monitor, row.label, arrow = true) { page = MenuPage.DISPLAYS }
                MenuRow.SHARE_CLIPBOARD -> StreamMenuRow(R.drawable.lucide_clipboard, row.label, checked = state.shareClipboard) {
                    state.shareClipboard = !state.shareClipboard; a.setShareClipboard(state.shareClipboard)
                }
                MenuRow.STREAM_STATS -> StreamMenuRow(R.drawable.lucide_chart_no_axes_column_increasing, row.label, checked = state.showPerf) { state.showPerf = !state.showPerf; close() }
                MenuRow.AUDIO -> StreamMenuRow(R.drawable.lucide_volume_2, row.label, arrow = true) { page = MenuPage.AUDIO }
                MenuRow.MORE_KEYS -> StreamMenuRow(R.drawable.lucide_command, row.label, arrow = true) { page = MenuPage.KEYS }
                MenuRow.HELP -> {
                    StreamMenuRow(R.drawable.lucide_circle_help, row.label) { context.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW,
                        android.net.Uri.parse(StreamMenuModel.HELP_URL))) }
                    MenuDivider()
                }
                MenuRow.DISCONNECT -> StreamMenuRow(R.drawable.lucide_log_out, row.label, value = StreamMenuModel.DISCONNECT_HINT, destructive = true) { a.disconnect() }
            }
            Text("Secpar " + com.pcremote.BuildConfig.VERSION_NAME,
                color = MenuColors.muted,
                style = androidx.compose.ui.text.TextStyle(
                    fontFamily = androidx.compose.ui.text.font.FontFamily.SansSerif,
                    fontWeight = FontWeight.Normal,
                    fontSize = MenuTokens.FOOTER_SIZE.sp,
                    lineHeight = 12.sp,
                    letterSpacing = 0.sp,
                    platformStyle = androidx.compose.ui.text.PlatformTextStyle(includeFontPadding = false),
                ),
                maxLines = 1,
                modifier = Modifier.padding(start = (MenuTokens.FOOTER_X - MenuTokens.PANEL_PADDING).dp, top = MenuTokens.FOOTER_TOP.dp))
        } else {
            StreamMenuRow(R.drawable.lucide_arrow_left, StreamMenuModel.backTitle(page)) { page = StreamMenuModel.backTarget(page) }
            MenuDivider()
            when (page) {
                MenuPage.KEYS -> {
                    StreamMenuRow(R.drawable.lucide_corner_down_left, "Escape") { a.keys(chord(listOf(VK_ESC))); close() }
                    StreamMenuRow(R.drawable.lucide_panels_top_left, "Windows key") { a.shortcut(Shortcut.WIN); close() }
                    StreamMenuRow(R.drawable.lucide_arrow_left_right, "Switch app", value = "Alt+Tab") { a.shortcut(Shortcut.ALT_TAB); close() }
                    StreamMenuRow(R.drawable.lucide_copy, "Copy", value = "Ctrl+C") { a.shortcut(Shortcut.COPY); close() }
                    StreamMenuRow(R.drawable.lucide_clipboard_paste, "Paste", value = "Ctrl+V") { a.shortcut(Shortcut.PASTE); close() }
                    StreamMenuRow(R.drawable.lucide_panels_top_left, "Show desktop") { a.keys(chord(listOf(0x5B, 0x44))); close() }
                    StreamMenuRow(R.drawable.lucide_monitor, "Task Manager") { a.keys(chord(listOf(0x11, 0x10, VK_ESC))); close() }
                    StreamMenuRow(R.drawable.lucide_x, "Close window", value = "Alt+F4") { a.keys(chord(listOf(0x12, 0x73))); close() }
                    StreamMenuRow(R.drawable.lucide_music_2, "Media keys", arrow = true) { page = MenuPage.MEDIA }
                }
                MenuPage.MEDIA -> {
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
                MenuPage.RESOLUTION -> STREAM_SIZES.forEach { (w, h) -> StreamMenuRow(R.drawable.lucide_monitor, sizeLabel(w, h), checked = w == state.sizeW && h == state.sizeH) {
                    if (w != state.sizeW || h != state.sizeH) { state.sizeW = w; state.sizeH = h; a.selectSize(w, h) }
                    page = StreamMenuModel.pageAfterPick()
                } }
                MenuPage.DISPLAYS -> state.displays.forEach { display -> StreamMenuRow(R.drawable.lucide_monitor, display.name, checked = display.id == state.display) {
                    state.display = display.id; a.selectDisplay(display.id); page = StreamMenuModel.pageAfterPick()
                } }
                MenuPage.AUDIO -> {
                    val body = StreamMenuModel.audioBody(state.audioOn, audioDetails)
                    if (body.isNotEmpty()) Text(body, color = if (state.audioOn) MenuColors.text else MenuColors.muted, fontSize = 12.sp,
                        modifier = Modifier.fillMaxWidth().padding(horizontal = MenuTokens.ICON_X.dp, vertical = 8.dp))
                    if (state.audioOn) {
                        StreamMenuRow(R.drawable.lucide_clipboard, StreamMenuModel.AUDIO_COPY_DETAILS) { a.copyText(audioDetails) }
                        StreamMenuRow(R.drawable.lucide_volume_2, StreamMenuModel.AUDIO_REOPEN) { a.reopenAudio() }
                    }
                }
                MenuPage.MAIN -> Unit
            }
        }
    }
}

@Composable
internal fun StreamMenuRow(icon: Int, label: String, value: String? = null, arrow: Boolean = false,
    checked: Boolean = false, destructive: Boolean = false, onClick: () -> Unit) {
    val color = if (destructive) MenuColors.danger else MenuColors.text
    val source = remember { MutableInteractionSource() }
    val pressed by source.collectIsPressedAsState()
    val hovered by source.collectIsHoveredAsState()
    val trailing = checked || arrow
    Row(Modifier.fillMaxWidth().height(MenuTokens.ROW_HEIGHT.dp).semantics { contentDescription = label }
        .clip(RoundedCornerShape(MenuTokens.ROW_RADIUS.dp)).background(if (pressed || hovered) MenuColors.rowPressed else Color.Transparent)
        .hoverable(source).clickable(source, null, onClick = onClick).padding(start = MenuTokens.ICON_X.dp),
        verticalAlignment = Alignment.CenterVertically) {
        Icon(painterResource(icon), null, tint = color, modifier = Modifier.size(MenuTokens.ICON.dp))
        Spacer(Modifier.width((MenuTokens.LABEL_X - MenuTokens.ICON_X - MenuTokens.ICON).dp))
        Text(label, color = color, fontSize = 14.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
        if (value != null) Text(value, color = MenuColors.muted, fontSize = 12.sp, maxLines = 1)
        if (trailing) {
            // the value ends 30 from the row edge; the 18 icon is centred 15 from it
            val end = (MenuTokens.TRAILING_CENTRE - MenuTokens.TRAILING_ICON / 2).dp
            val gap = (MenuTokens.VALUE_END_WITH_TRAILING - MenuTokens.TRAILING_ICON - MenuTokens.TRAILING_CENTRE + MenuTokens.TRAILING_ICON / 2).dp
            Spacer(Modifier.width(gap))
            Icon(painterResource(if (checked) R.drawable.lucide_check else R.drawable.lucide_chevron_right), null,
                tint = if (checked) MenuColors.check else MenuColors.muted, modifier = Modifier.size(MenuTokens.TRAILING_ICON.dp))
            Spacer(Modifier.width(end))
        } else if (value != null) Spacer(Modifier.width(MenuTokens.VALUE_END.dp))
    }
}

@Composable
private fun KeyboardBar(state: ToolbarState, a: ToolbarActions) {
    val c = LocalMidnight.current
    val haptics = rememberHaptics()
    var fn by remember { mutableStateOf(false) }
    @Suppress("UNUSED_EXPRESSION") state.barTick // recompose when modifiers change

    @Composable
    fun Cap(text: String? = null, @androidx.annotation.DrawableRes icon: Int? = null, mod: Mod? = null, onClick: () -> Unit) {
        val st = mod?.let { state.bar.state(it) } ?: ModState.OFF
        val bg = when (st) { ModState.LOCKED -> c.accent; ModState.ONE_SHOT -> c.accent.copy(alpha = 0.3f); else -> c.surfaceHigh }
        val fg = if (st == ModState.LOCKED) c.onAccent else c.text
        Box(
            Modifier.height(40.dp).widthIn(min = 44.dp).clip(MidnightShapes.control).background(bg)
                .clickable { haptics.tick(); onClick() }.padding(horizontal = 12.dp),
            contentAlignment = Alignment.Center,
        ) {
            if (icon != null) Icon(painterResource(icon), text, tint = fg, modifier = Modifier.size(20.dp))
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
            Cap(icon = R.drawable.lucide_corner_down_left) { fn = false }
        } else {
            Cap("Esc") { press(VK_ESC) }
            Cap(icon = R.drawable.lucide_arrow_right_to_line) { press(VK_TAB) }
            Cap("Ctrl", mod = Mod.CTRL) { tapMod(Mod.CTRL) }
            Cap("Alt", mod = Mod.ALT) { tapMod(Mod.ALT) }
            Cap("Shift", mod = Mod.SHIFT) { tapMod(Mod.SHIFT) }
            Cap(icon = R.drawable.lucide_app_window, mod = Mod.WIN) { tapMod(Mod.WIN) }
            Cap(icon = R.drawable.lucide_chevron_left) { press(VK_LEFT) }
            Cap(icon = R.drawable.lucide_chevron_up) { press(VK_UP) }
            Cap(icon = R.drawable.lucide_chevron_down) { press(VK_DOWN) }
            Cap(icon = R.drawable.lucide_chevron_right) { press(VK_RIGHT) }
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
            Text(stringResource(R.string.pcr_connecting_to, pcName ?: "computer"), color = c.text, style = MaterialTheme.typography.headlineSmall)
            Spacer(Modifier.height(8.dp))
            AnimatedContent(stage, transitionSpec = { fadeIn() togetherWith fadeOut() }, label = "stage") {
                Text(it, color = c.textDim, style = MaterialTheme.typography.bodyMedium)
            }
            Spacer(Modifier.height(32.dp))
            GhostButton(stringResource(R.string.pcr_cancel), cancel)
        }
    }
}
