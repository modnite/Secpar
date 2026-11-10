package com.pcremote.account

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
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

/** The Friends tab. Same strings and calls as the desktop client. */
@Composable
fun FriendsPage(state: AccountUi, acts: AccountActions, modifier: Modifier = Modifier, alertsOn: Boolean = false) {
    val c = LocalMidnight.current
    val ctx = LocalContext.current
    var query by remember { mutableStateOf("") }
    var findOpen by remember { mutableStateOf(false) }
    val permission = androidx.activity.compose.rememberLauncherForActivityResult(
        androidx.activity.result.contract.ActivityResultContracts.RequestPermission()) { granted -> acts.setAlerts(granted) }
    LaunchedEffect(query, findOpen, state.user?.id) {
        if (findOpen && state.user != null) {
            kotlinx.coroutines.delay(300)
            acts.search(query)
        }
    }
    val user = state.user
    Column(modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp, vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp)) {
        PageHeader("Friends") {
            if (user != null) {
                Text(user.handle, color = c.textDim, style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.widthIn(max = 120.dp))
                GhostButton("Sign out", acts.signOut, compact = true, enabled = !state.busy)
            }
        }
        if (user == null) {
            Text(FriendsModel.SIGN_IN_PROMPT, color = c.textDim, style = MaterialTheme.typography.bodyMedium)
            SignInForm(acts, state.busy)
            return@Column
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(FriendsModel.youAre(user.handle), color = c.text, style = MaterialTheme.typography.bodyMedium, maxLines = 1,
                overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
            GhostButton("Refresh", acts.refresh, compact = true, enabled = !state.working)
        }
        FlatCheckbox(user.visible, FriendsModel.SHOW_ME, { acts.setVisible(it) })
        FlatCheckbox(alertsOn, FriendsModel.NOTIFY, { on ->
            if (!on) acts.setAlerts(false)
            else if (android.os.Build.VERSION.SDK_INT >= 33 &&
                androidx.core.content.ContextCompat.checkSelfPermission(ctx, android.Manifest.permission.POST_NOTIFICATIONS) !=
                android.content.pm.PackageManager.PERMISSION_GRANTED) permission.launch(android.Manifest.permission.POST_NOTIFICATIONS)
            else acts.setAlerts(true)
        })
        FriendsModel.waiting(state.requests, user.id).forEach { request ->
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(FriendsModel.requestText(request), color = c.text, style = MaterialTheme.typography.bodyMedium, maxLines = 2,
                    overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                GhostButton(FriendsModel.DENY, { acts.answer(Answer.DENY_CONNECTION, request.id) }, compact = true, enabled = !state.working)
                PrimaryButton(FriendsModel.ALLOW, { acts.answer(Answer.ALLOW_CONNECTION, request.id) }, compact = true, enabled = !state.working)
            }
        }
        Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable { findOpen = !findOpen }, verticalAlignment = Alignment.CenterVertically) {
            Icon(painterResource(if (findOpen) R.drawable.lucide_chevron_down else R.drawable.lucide_chevron_right), null, tint = c.textDim,
                modifier = Modifier.size(18.dp))
            Spacer(Modifier.width(6.dp))
            Text(FriendsModel.FIND_PEOPLE, color = c.text, style = MaterialTheme.typography.titleSmall)
        }
        if (findOpen) {
            FlatField(query, { query = it }, FriendsModel.SEARCH_HINT, Modifier.fillMaxWidth())
            if (state.people.isEmpty()) Text(FriendsModel.NO_ONE, color = c.textDim, style = MaterialTheme.typography.bodyMedium)
            state.people.forEach { person ->
                Row(Modifier.fillMaxWidth().heightIn(min = 40.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("@${person.handle}", color = c.text, style = MaterialTheme.typography.bodyMedium, maxLines = 1, overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f))
                    val label = FriendsModel.personLabel(person)
                    if (label == null) PrimaryButton(FriendsModel.ADD, { acts.friend(FriendAction.INVITE, person.handle) }, compact = true, enabled = !state.working)
                    else Text(label, color = c.textDim, style = MaterialTheme.typography.bodySmall)
                }
            }
        }
        FriendComputersList(state, acts)
        if (state.friends.isEmpty()) Text(FriendsModel.EMPTY, color = c.textDim, style = MaterialTheme.typography.bodyMedium)
        state.friends.forEachIndexed { index, friend ->
            if (index > 0) HorizontalDivider(color = c.border)
            FriendRow(friend, user.id, state.working, acts)
        }
        if (state.blocked.isNotEmpty()) BlockedList(state.blocked, state.working, acts)
        state.note?.let { Text(it, color = c.textDim, style = MaterialTheme.typography.bodySmall) }
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
            FriendLabel.FRIEND -> Text(FriendsModel.summary(friend), color = c.textDim, style = MaterialTheme.typography.bodySmall)
            else -> Text(FriendsModel.labelText(label), color = c.textDim, style = MaterialTheme.typography.bodySmall)
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
