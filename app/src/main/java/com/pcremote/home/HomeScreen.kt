package com.pcremote.home

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.pcremote.BuildConfig
import com.pcremote.R
import com.pcremote.account.AccountActions
import com.pcremote.account.AccountUi
import com.pcremote.account.OwnedComputer
import com.pcremote.account.SignInPanel
import com.pcremote.agent.AgentConnection
import com.pcremote.modes.rememberUnlocker
import com.pcremote.nav.Mode
import com.pcremote.ui.components.*
import com.pcremote.ui.theme.LocalMidnight
import com.pcremote.ui.theme.MidnightShapes

@androidx.annotation.DrawableRes
fun Mode.iconRes(): Int = when (this) {
    Mode.SCREEN -> R.drawable.lucide_monitor
    Mode.REMOTE -> R.drawable.lucide_touchpad
    Mode.MEDIA -> R.drawable.lucide_music
    Mode.SLIDES -> R.drawable.lucide_presentation
    Mode.PAD -> R.drawable.lucide_gamepad_2
    Mode.FILES -> R.drawable.lucide_folder
}

fun Mode.label(): Int = when (this) {
    Mode.SCREEN -> R.string.pcr_mode_screen
    Mode.REMOTE -> R.string.pcr_mode_remote
    Mode.MEDIA -> R.string.pcr_mode_media
    Mode.SLIDES -> R.string.pcr_mode_slides
    Mode.PAD -> R.string.pcr_mode_pad
    Mode.FILES -> R.string.pcr_mode_files
}

enum class HomeTab(val label: Int) {
    COMPUTERS(R.string.pcr_tab_computers), FRIENDS(R.string.pcr_tab_friends), SETTINGS(R.string.pcr_tab_settings)
}

class HomeActions(
    val open: (PcCardUi, Mode) -> Unit,
    val primary: (PcCardUi) -> Unit,
    val addPc: () -> Unit,
    val refresh: () -> Unit,
    val rename: (PcCardUi, String) -> Unit,
    val remove: (PcCardUi) -> Unit,
    val send: (PcCardUi, com.pcremote.agent.Msg) -> Unit = { _, _ -> },
    val connection: (PcCardUi) -> AgentConnection? = { null },
    val updates: () -> Unit = {},
)

private val PageMargin = 20.dp

/**
 * Home: logo bar, tab strip, and the page of the selected tab. The Friends and Settings pages come
 * in as slots so this file only knows about computers.
 */
@Composable
fun HomeScreen(
    cards: List<PcCardUi>, thumbs: Map<String, ImageBitmap>, implemented: Set<Mode>, a: HomeActions,
    account: AccountUi = AccountUi(), acts: AccountActions = AccountActions(),
    updateAvailable: Boolean = false, initialTab: HomeTab = HomeTab.COMPUTERS,
    friends: @Composable () -> Unit = {}, settings: @Composable () -> Unit = {},
) {
    val c = LocalMidnight.current
    var tab by rememberSaveable(initialTab) { mutableStateOf(initialTab) }
    Column(Modifier.fillMaxSize().background(c.bg).safeDrawingPadding()) {
        Header(updateAvailable, a.updates)
        TabStrip(tab) { tab = it }
        if (tab != HomeTab.SETTINGS) account.error?.let { ErrorBar(it, acts.dismissError) }
        Box(Modifier.fillMaxWidth().weight(1f), contentAlignment = Alignment.TopCenter) {
            Box(Modifier.widthIn(max = 760.dp).fillMaxSize()) {
                when (tab) {
                    HomeTab.COMPUTERS -> ComputersPage(cards, thumbs, implemented, a, account, acts)
                    HomeTab.FRIENDS -> friends()
                    HomeTab.SETTINGS -> settings()
                }
            }
        }
    }
}

@Composable
private fun Header(updateAvailable: Boolean, onUpdates: () -> Unit) {
    val c = LocalMidnight.current
    Row(Modifier.fillMaxWidth().padding(horizontal = PageMargin, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        SecparWordmark(Modifier.width(112.dp).height(27.dp))
        Spacer(Modifier.width(8.dp))
        Text(BuildConfig.VERSION_NAME, color = c.textDim, fontSize = 11.sp, maxLines = 1)
        Spacer(Modifier.weight(1f))
        GhostButton(stringResource(if (updateAvailable) R.string.pcr_update_available else R.string.pcr_updates), onUpdates, compact = true)
    }
}

/** Three text tabs, 100 x 36, the selected one white with a 2dp accent underline. 48dp high to touch. */
@Composable
private fun TabStrip(selected: HomeTab, onSelect: (HomeTab) -> Unit) {
    val c = LocalMidnight.current
    Row(Modifier.fillMaxWidth().padding(horizontal = PageMargin - 8.dp)) {
        HomeTab.entries.forEach { t ->
            val on = t == selected
            Box(Modifier.height(48.dp).clickable(role = androidx.compose.ui.semantics.Role.Tab) { onSelect(t) }.padding(horizontal = 8.dp),
                contentAlignment = Alignment.Center) {
                Box(Modifier.width(84.dp).height(36.dp)) {
                    Text(stringResource(t.label), color = if (on) c.text else c.textDim, style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier.align(Alignment.Center))
                    if (on) Box(Modifier.align(Alignment.BottomCenter).fillMaxWidth().height(2.dp).background(c.accent))
                }
            }
        }
    }
}

/** The error text, Copy error and Dismiss. */
@Composable
private fun ErrorBar(message: String, dismiss: () -> Unit) {
    val c = LocalMidnight.current
    val clipboard = LocalClipboardManager.current
    Row(Modifier.fillMaxWidth().padding(horizontal = PageMargin), verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(message, color = c.bad, style = MaterialTheme.typography.bodySmall, modifier = Modifier.weight(1f).padding(vertical = 8.dp))
        GhostButton(stringResource(R.string.pcr_copy_error), { clipboard.setText(AnnotatedString(message)) }, compact = true)
        GhostButton(stringResource(R.string.pcr_dismiss), dismiss, compact = true)
    }
}

@Composable
private fun ComputersPage(
    cards: List<PcCardUi>, thumbs: Map<String, ImageBitmap>, implemented: Set<Mode>, a: HomeActions,
    account: AccountUi, acts: AccountActions,
) {
    val c = LocalMidnight.current
    val rows = remember(cards, account.computers) { ComputerList.rows(cards, account.computers) }
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(start = PageMargin, end = PageMargin, top = 8.dp, bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp)) {
        item("title") {
            PageHeader(stringResource(R.string.pcr_tab_computers)) {
                GhostButton(stringResource(R.string.pcr_add_pc), a.addPc, compact = true)
                if (account.user == null && account.approval == null) {
                    GhostButton(stringResource(R.string.pcr_sign_in), acts.signIn, compact = true, enabled = account.ready && !account.busy)
                }
            }
        }
        account.user?.let { user ->
            item("account") {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(user.handle, color = c.textDim, style = MaterialTheme.typography.bodyMedium, maxLines = 1, overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f))
                    GhostButton(stringResource(R.string.pcr_refresh), acts.refresh, compact = true, enabled = account.connecting == null)
                    GhostButton(stringResource(R.string.pcr_sign_out), acts.signOut, compact = true)
                }
            }
        }
        if (account.user == null) account.approval?.let { item("approval") { SignInPanel(it, acts) } }
        if (rows.isEmpty()) item("empty") { EmptyState(a.addPc) }
        items(rows, key = { it.key }) { row ->
            when (row) {
                is ComputerRow.Saved -> SavedCard(row.card, thumbs[row.card.id], implemented, a)
                is ComputerRow.FromAccount -> AccountCard(row.computer, account.connecting, acts)
            }
        }
    }
}

@Composable
private fun EmptyState(addPc: () -> Unit) {
    val c = LocalMidnight.current
    Column(Modifier.fillMaxWidth().padding(top = 8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(stringResource(R.string.pcr_no_pcs_title), color = c.textDim, style = MaterialTheme.typography.bodyMedium)
        PrimaryButton(stringResource(R.string.pcr_ob_scan), addPc, icon = R.drawable.lucide_scan_qr_code)
    }
}

@Composable
private fun statusColor(s: ComputerStatus): Color {
    val c = LocalMidnight.current
    return when (s) {
        ComputerStatus.ONLINE -> c.ok
        ComputerStatus.STREAMING -> c.accent
        ComputerStatus.LOCKED, ComputerStatus.CHECKING, ComputerStatus.HOST_UPDATE_REQUIRED -> c.warn
        ComputerStatus.OFFLINE -> c.textDim
    }
}

@Composable
private fun statusText(s: ComputerStatus) = stringResource(when (s) {
    ComputerStatus.ONLINE -> R.string.pcr_status_online
    ComputerStatus.OFFLINE -> R.string.pcr_status_offline
    ComputerStatus.STREAMING -> R.string.pcr_status_streaming
    ComputerStatus.LOCKED -> R.string.pcr_status_locked
    ComputerStatus.CHECKING -> R.string.pcr_checking
    ComputerStatus.HOST_UPDATE_REQUIRED -> R.string.pcr_host_update_required
})

/** Dot, name 14 bold, status 12 muted, platform at the right. */
@Composable
private fun CardTitle(name: String, platform: String, status: ComputerStatus, endPad: androidx.compose.ui.unit.Dp = 0.dp) {
    val c = LocalMidnight.current
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top) {
        Box(Modifier.padding(top = 6.dp)) { StatusDot(statusColor(status)) }
        Spacer(Modifier.width(8.dp))
        Column(Modifier.weight(1f)) {
            Text(name, color = c.text, style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(statusText(status), color = c.textDim, style = MaterialTheme.typography.bodySmall, maxLines = 1)
        }
        if (platform.isNotEmpty()) Text(platform, color = c.textDim, style = MaterialTheme.typography.bodySmall, maxLines = 1,
            modifier = Modifier.padding(start = 8.dp, top = 2.dp))
        Spacer(Modifier.width(endPad))
    }
}

@Composable
private fun CardFrame(content: @Composable ColumnScope.() -> Unit) {
    val c = LocalMidnight.current
    Column(Modifier.fillMaxWidth().clip(MidnightShapes.card).background(c.surface).border(1.dp, c.border, MidnightShapes.card).padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp), content = content)
}

@Composable
private fun AccountCard(computer: OwnedComputer, connecting: String?, acts: AccountActions) {
    val status = ComputerList.status(computer)
    CardFrame {
        CardTitle(computer.name, ComputerList.platform(computer.platform), status)
        if (connecting == computer.id) GhostButton(stringResource(R.string.pcr_cancel), acts.cancelConnect)
        else PrimaryButton(stringResource(R.string.pcr_connect), { acts.connect(computer) }, enabled = computer.linked && connecting == null)
    }
}

@Composable
private fun SavedCard(card: PcCardUi, thumb: ImageBitmap?, implemented: Set<Mode>, a: HomeActions) {
    val conn = a.connection(card)
    val unlock = rememberUnlocker(card.id, { m -> conn?.sendIfOnline(m) == true }, conn?.incoming)
    val modes = card.modes.filter { it in implemented && it != Mode.SCREEN }
    var renaming by remember { mutableStateOf(false) }
    var confirmForget by remember { mutableStateOf(false) }
    var power by remember { mutableStateOf(false) }
    if (power) com.pcremote.modes.PowerSheet(card.name, { a.send(card, it) }) { power = false }
    if (renaming) RenameDialog(card.name, { renaming = false }) { a.rename(card, it) }
    val items = buildList {
        if (card.canPower) add(MenuItem(R.drawable.lucide_power, stringResource(R.string.pcr_power)) { power = true })
        if (card.primary != PrimaryAction.OPEN) add(MenuItem(R.drawable.lucide_zap, stringResource(R.string.pcr_wake)) { a.primary(card.copy(primary = PrimaryAction.WAKE)) })
        add(MenuItem(R.drawable.lucide_pencil, stringResource(R.string.pcr_rename)) { renaming = true })
        add(MenuItem(R.drawable.lucide_trash_2, stringResource(if (confirmForget) R.string.pcr_pw_tap_again else R.string.pcr_forget_device),
            destructive = true, keepOpen = !confirmForget) {
            if (confirmForget) a.remove(card) else confirmForget = true
        })
    }
    Box {
    CardFrame {
        CardTitle(card.name, card.os.substringBefore(' '), ComputerList.status(card), endPad = 36.dp)
        // The preview, then one strip: Connect first, the modes after, each a glyph with no label.
        Column(Modifier.widthIn(max = 360.dp).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            if (card.online && thumb != null) Preview(thumb, Modifier.fillMaxWidth())
            if (card.online) Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                GlyphTile(R.drawable.lucide_monitor, stringResource(R.string.pcr_connect), Modifier.weight(1f), primary = true) { a.primary(card) }
                modes.forEach { m -> GlyphTile(m.iconRes(), stringResource(m.label()), Modifier.weight(1f)) { a.open(card, m) } }
                if (card.locked && card.canUnlock) GlyphTile(R.drawable.lucide_fingerprint, stringResource(R.string.pcr_unlock), Modifier.weight(1f), onClick = unlock)
            } else if (card.primary == PrimaryAction.WAKE) PrimaryButton(stringResource(R.string.pcr_wake), { a.primary(card) })
        }
    }
    // The 48dp touch target sits over the card's top right corner; the dots line up with the name row.
    AnchoredMenu(stringResource(R.string.pcr_more), items, Modifier.align(Alignment.TopEnd).offset(y = (-2).dp))
    }
}

/** The computer's screen as a 16:9 preview. Shown only while a thumbnail exists. */
@Composable
private fun Preview(thumb: ImageBitmap, modifier: Modifier) {
    val c = LocalMidnight.current
    Image(thumb, null, modifier.aspectRatio(16f / 9f).clip(MidnightShapes.control)
        .border(1.dp, c.border, MidnightShapes.control), contentScale = ContentScale.Crop)
}

/** A 44dp tall button holding one glyph. The label is only for screen readers. */
@Composable
private fun GlyphTile(@androidx.annotation.DrawableRes icon: Int, label: String, modifier: Modifier, primary: Boolean = false, onClick: () -> Unit) {
    val c = LocalMidnight.current
    val haptics = com.pcremote.ui.theme.rememberHaptics()
    Box(modifier.height(44.dp).clip(MidnightShapes.control).background(if (primary) c.accent else c.surfaceHigh)
        .then(if (primary) Modifier else Modifier.border(1.dp, c.controlBorder, MidnightShapes.control))
        .clickable(role = androidx.compose.ui.semantics.Role.Button, onClickLabel = label) { haptics.tick(); onClick() }
        .semantics { contentDescription = label }, contentAlignment = Alignment.Center) {
        Icon(painterResource(icon), null, tint = if (primary) c.onAccent else c.text, modifier = Modifier.size(22.dp))
    }
}

@Composable
private fun RenameDialog(current: String, onDismiss: () -> Unit, onSave: (String) -> Unit) {
    var name by remember { mutableStateOf(current) }
    FlatDialog(stringResource(R.string.pcr_rename), onDismiss, actions = {
        GhostButton(stringResource(R.string.pcr_cancel), onDismiss, compact = true)
        PrimaryButton(stringResource(R.string.pcr_save), { onSave(name.trim().ifEmpty { current }); onDismiss() }, compact = true)
    }) {
        FlatField(name, { name = it }, current, Modifier.fillMaxWidth())
    }
}
