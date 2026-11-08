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

/** "Confirm this code in your browser", the code, and what to do with it. Shared by Computers and Friends. */
@Composable
fun SignInPanel(approval: DeviceSignIn, acts: AccountActions, modifier: Modifier = Modifier) {
    val c = LocalMidnight.current
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(FriendsModel.CONFIRM_IN_BROWSER, color = c.text, style = MaterialTheme.typography.bodyMedium)
        Text(approval.userCode, color = c.text, style = MaterialTheme.typography.headlineMedium, fontFamily = FontFamily.Monospace)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            PrimaryButton("Open browser", acts.openBrowser)
            GhostButton("Cancel", acts.cancelSignIn)
        }
    }
}

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
            val approval = state.approval
            if (approval != null) SignInPanel(approval, acts)
            else {
                Text(FriendsModel.SIGN_IN_PROMPT, color = c.textDim, style = MaterialTheme.typography.bodyMedium)
                PrimaryButton("Sign in", acts.signIn, enabled = state.ready && !state.busy)
                Text(FriendsModel.CREATE_ACCOUNT, color = c.accent, style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.heightIn(min = 48.dp).wrapContentHeight(Alignment.CenterVertically).clickable {
                        runCatching { ctx.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(FriendsModel.CREATE_ACCOUNT_URL))) }
                    })
            }
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
        if (state.friends.isEmpty()) Text(FriendsModel.EMPTY, color = c.textDim, style = MaterialTheme.typography.bodyMedium)
        state.friends.forEachIndexed { index, friend ->
            if (index > 0) HorizontalDivider(color = c.border)
            FriendRow(friend, user.id, state.working, acts)
        }
        if (state.blocked.isNotEmpty()) BlockedList(state.blocked, state.working, acts)
        state.note?.let { Text(it, color = c.textDim, style = MaterialTheme.typography.bodySmall) }
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
            MenuItem(R.drawable.lucide_keyboard, if (friend.input == "full") "View only" else "Allow keyboard and mouse") {
                acts.friendSetting(friend.id, if (friend.input == "full") "view" else "full", null)
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
