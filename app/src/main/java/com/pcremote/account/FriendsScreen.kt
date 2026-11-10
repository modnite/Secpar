package com.pcremote.account

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.pcremote.R
import com.pcremote.ui.components.*
import com.pcremote.ui.theme.LocalMidnight

private val AvatarFill = Color(0xFF203036)

/** Friends use the same account and permissions; this page only arranges their controls. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FriendsPage(state: AccountUi, acts: AccountActions, modifier: Modifier = Modifier, alertsOn: Boolean = false) {
    val c = LocalMidnight.current
    val ctx = LocalContext.current
    var query by remember { mutableStateOf("") }
    var findOpen by remember { mutableStateOf(false) }
    var preferencesOpen by remember { mutableStateOf(false) }
    val permission = androidx.activity.compose.rememberLauncherForActivityResult(
        androidx.activity.result.contract.ActivityResultContracts.RequestPermission()) { granted -> acts.setAlerts(granted) }
    LaunchedEffect(query, findOpen, state.user?.id) {
        if (findOpen && state.user != null) { kotlinx.coroutines.delay(300); acts.search(query) }
    }
    val user = state.user
    Column(modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp, vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if (user == null) {
            Text("Sign in on Computers.", color = c.textDim, style = MaterialTheme.typography.bodyMedium)
            return@Column
        }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            PrimaryButton(if (findOpen) "Close search" else FriendsModel.FIND_PEOPLE, { findOpen = !findOpen },
                modifier = Modifier.weight(1f), compact = true)
            GhostButton("Settings", { preferencesOpen = true }, compact = true)
        }
        if (findOpen) FriendSection {
            FlatField(query, { query = it }, FriendsModel.SEARCH_HINT, Modifier.fillMaxWidth())
            if (state.people.isEmpty()) Text(FriendsModel.NO_ONE, color = c.textDim, style = MaterialTheme.typography.bodyMedium)
            state.people.forEach { person ->
                Row(Modifier.fillMaxWidth().heightIn(min = 40.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("@${person.handle}", color = c.text, style = MaterialTheme.typography.bodyMedium, maxLines = 1,
                        overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                    val label = FriendsModel.personLabel(person)
                    if (label == null) PrimaryButton(FriendsModel.ADD, { acts.friend(FriendAction.INVITE, person.handle) }, compact = true, enabled = !state.working)
                    else Text(label, color = c.textDim, style = MaterialTheme.typography.bodySmall)
                }
            }
        }
        val waiting = FriendsModel.waiting(state.requests, user.id)
        if (waiting.isNotEmpty()) FriendSection {
            Text("Connection requests", color = c.text, style = MaterialTheme.typography.titleSmall)
            waiting.forEach { request ->
                Text(FriendsModel.requestText(request), color = c.text, style = MaterialTheme.typography.bodyMedium)
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    GhostButton(FriendsModel.DENY, { acts.answer(Answer.DENY_CONNECTION, request.id) }, modifier = Modifier.weight(1f), compact = true, enabled = !state.working)
                    PrimaryButton(FriendsModel.ALLOW, { acts.answer(Answer.ALLOW_CONNECTION, request.id) }, modifier = Modifier.weight(1f), compact = true, enabled = !state.working)
                }
            }
        }
        val requests = state.friends.filterNot { it.accepted }
        if (requests.isNotEmpty()) FriendSection {
            Text("Friend requests", color = c.text, style = MaterialTheme.typography.titleSmall)
            requests.forEach { FriendRow(it, user.id, state.working, acts) }
        }
        val friends = state.friends.filter { it.accepted }
        if (friends.isEmpty()) Text(FriendsModel.EMPTY, color = c.textDim, style = MaterialTheme.typography.bodyMedium)
        else FriendSection {
            friends.forEachIndexed { index, friend ->
                if (index > 0) HorizontalDivider(color = c.border)
                FriendRow(friend, user.id, state.working, acts)
            }
        }
        if (state.friendComputers.isNotEmpty()) FriendSection { FriendComputersList(state, acts) }
        if (state.blocked.isNotEmpty()) FriendSection { BlockedList(state.blocked, state.working, acts) }
        state.note?.let { Text(it, color = c.textDim, style = MaterialTheme.typography.bodySmall) }
    }
    if (preferencesOpen && user != null) ModalBottomSheet(onDismissRequest = { preferencesOpen = false },
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), containerColor = c.surface) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 20.dp).padding(bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Friend settings", color = c.text, style = MaterialTheme.typography.titleMedium)
            FlatCheckbox(user.visible, FriendsModel.SHOW_ME, { acts.setVisible(it) })
            FlatCheckbox(alertsOn, FriendsModel.NOTIFY, { on ->
                if (!on) acts.setAlerts(false)
                else if (android.os.Build.VERSION.SDK_INT >= 33 &&
                    androidx.core.content.ContextCompat.checkSelfPermission(ctx, android.Manifest.permission.POST_NOTIFICATIONS) !=
                    android.content.pm.PackageManager.PERMISSION_GRANTED) permission.launch(android.Manifest.permission.POST_NOTIFICATIONS)
                else acts.setAlerts(true)
            })
            GhostButton("Done", { preferencesOpen = false }, modifier = Modifier.fillMaxWidth())
        }
    }
}

@Composable
private fun FriendSection(content: @Composable ColumnScope.() -> Unit) {
    val c = LocalMidnight.current
    val shape = RoundedCornerShape(8.dp)
    Column(Modifier.fillMaxWidth().clip(shape).background(c.surface).border(1.dp, c.border, shape).padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp), content = content)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun FriendGlyph(@androidx.annotation.DrawableRes icon: Int, label: String, allowed: Boolean = true) {
    val c = LocalMidnight.current
    TooltipBox(positionProvider = TooltipDefaults.rememberPlainTooltipPositionProvider(),
        tooltip = { PlainTooltip { Text(label) } }, state = rememberTooltipState()) {
        Icon(painterResource(icon), label, tint = if (allowed) c.accent else c.textDim, modifier = Modifier.size(18.dp))
    }
}

/** Computers of friends that are online and let friends connect. Connect opens the permission choices. */
@Composable
private fun FriendComputersList(state: AccountUi, acts: AccountActions) {
    if (state.friendComputers.isEmpty()) return
    val c = LocalMidnight.current
    var picking by remember { mutableStateOf<String?>(null) }
    var choices by remember { mutableStateOf(GuestChoices()) }
    Text(FriendsConnect.HEADING, color = c.text, style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(top = 8.dp))
    state.friendComputers.forEach { computer ->
        val progress = state.guest?.takeIf { it.computerId == computer.id }
        Row(Modifier.fillMaxWidth().heightIn(min = 48.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Column(Modifier.weight(1f)) {
                Text(computer.name, color = c.text, style = MaterialTheme.typography.bodyMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(FriendsConnect.owner(computer.ownerHandle), color = c.textDim, style = MaterialTheme.typography.bodySmall, maxLines = 1,
                    overflow = TextOverflow.Ellipsis)
            }
            if (progress != null) GhostButton(FriendsConnect.CANCEL, acts.cancelFriendConnect, compact = true)
            else PrimaryButton(FriendsConnect.CONNECT, { choices = GuestChoices(); picking = if (picking == computer.id) null else computer.id },
                compact = true, enabled = state.guest == null)
        }
        if (progress != null) Text(FriendsConnect.phaseText(progress), color = c.textDim, style = MaterialTheme.typography.bodyMedium)
        else if (picking == computer.id) {
            FlatCheckbox(choices.view, FriendsConnect.VIEW, { choices = choices.copy(view = it) })
            FlatCheckbox(choices.audio, FriendsConnect.AUDIO, { choices = choices.copy(audio = it) })
            FlatCheckbox(choices.input, FriendsConnect.INPUT, { choices = choices.copy(input = it) })
            FlatCheckbox(choices.controller, FriendsConnect.CONTROLLER, { choices = choices.copy(controller = it) })
            FlatCheckbox(choices.clipboard, FriendsConnect.CLIPBOARD, { choices = choices.copy(clipboard = it) })
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                PrimaryButton(FriendsConnect.ASK, { picking = null; acts.connectFriend(computer, choices.permissions(), com.pcremote.nav.Mode.SCREEN) },
                    enabled = choices.permissions().isNotEmpty())
                GhostButton(FriendsConnect.CANCEL, { picking = null })
            }
        }
        HorizontalDivider(color = c.border)
    }
}

@Composable
private fun FriendRow(friend: Friend, ownId: String, working: Boolean, acts: AccountActions) {
    val c = LocalMidnight.current
    val label = FriendsModel.label(friend, ownId)
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Box(Modifier.size(32.dp).clip(RoundedCornerShape(6.dp)).background(AvatarFill), contentAlignment = Alignment.Center) {
            Text(FriendsModel.initial(friend.handle), color = c.accent, style = MaterialTheme.typography.bodyMedium)
        }
        Text(friend.handle, color = c.text, style = MaterialTheme.typography.bodyMedium, maxLines = 1, overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f))
        when (label) {
            FriendLabel.ACCEPT -> PrimaryButton(FriendsModel.ACCEPT, { acts.friend(FriendAction.ACCEPT, friend.id) }, compact = true, enabled = !working)
            FriendLabel.FRIEND -> Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                FriendGlyph(R.drawable.lucide_keyboard, if (friend.keyboard) "Keyboard allowed" else "Keyboard blocked", friend.keyboard)
                FriendGlyph(R.drawable.lucide_mouse, if (friend.mouse) "Mouse allowed" else "Mouse blocked", friend.mouse)
                FriendGlyph(if (friend.connect == "always") R.drawable.lucide_circle_check else R.drawable.lucide_circle_help,
                    if (friend.connect == "always") "Connect any time" else "Ask first", friend.connect == "always")
            }
            else -> FriendGlyph(R.drawable.lucide_circle_help, FriendsModel.labelText(label), false)
        }
        val settings = if (friend.accepted) listOf(
            MenuItem(R.drawable.lucide_keyboard, if (friend.keyboard) "Block keyboard" else "Allow keyboard") {
                acts.friendInput(friend.id, !friend.keyboard, null)
            },
            MenuItem(R.drawable.lucide_mouse, if (friend.mouse) "Block mouse" else "Allow mouse") {
                acts.friendInput(friend.id, null, !friend.mouse)
            },
            MenuItem(R.drawable.lucide_circle_check, if (friend.connect == "always") "Ask me first" else "Connect any time") {
                acts.friendSetting(friend.id, null, if (friend.connect == "always") "ask" else "always")
            },
        ) else emptyList()
        AnchoredMenu(friend.handle, settings + listOf(
            MenuItem(R.drawable.lucide_ban, FriendsModel.BLOCK) { acts.friend(FriendAction.BLOCK, friend.id) },
            MenuItem(R.drawable.lucide_user_minus, FriendsModel.removeText(friend)) { acts.friend(FriendAction.REMOVE, friend.id) },
        ))
    }
}

@Composable
private fun BlockedList(blocked: List<AccountUser>, working: Boolean, acts: AccountActions) {
    val c = LocalMidnight.current
    var open by remember { mutableStateOf(false) }
    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable { open = !open }, verticalAlignment = Alignment.CenterVertically) {
        Icon(painterResource(if (open) R.drawable.lucide_chevron_down else R.drawable.lucide_chevron_right), null, tint = c.textDim,
            modifier = Modifier.size(18.dp))
        Spacer(Modifier.width(6.dp))
        Text(FriendsModel.BLOCKED_HEADING, color = c.text, style = MaterialTheme.typography.titleSmall)
    }
    if (open) blocked.forEach { person ->
        Row(Modifier.fillMaxWidth().padding(start = 24.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(person.handle, color = c.text, style = MaterialTheme.typography.bodyMedium, maxLines = 1, overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f))
            GhostButton(FriendsModel.UNBLOCK, { acts.friend(FriendAction.UNBLOCK, person.id) }, compact = true, enabled = !working)
        }
    }
}
