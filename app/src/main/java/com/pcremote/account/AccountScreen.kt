package com.pcremote.account

import android.content.Intent
import android.net.Uri
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.Computer
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import com.pcremote.ui.components.SecparWordmark
import com.pcremote.ui.theme.LocalMidnight
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

@Composable
fun AccountScreen(controller: AccountController, onBack: () -> Unit, onOpen: (String) -> Unit) {
    val state by controller.state.collectAsState()
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    var browserError by remember { mutableStateOf<String?>(null) }
    fun back() { controller.cancelConnection(); onBack() }
    BackHandler(onBack = ::back)
    DisposableEffect(controller) { onDispose { controller.cancelConnection() } }
    fun browser() {
        val approval = state.approval ?: return
        browserError = if (runCatching { ctx.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(approval.verificationUriComplete))) }.isSuccess)
            null else "No browser is available. Open accounts.secpar.win/activate and enter the code."
    }
    LaunchedEffect(state.approval) { if (state.approval != null) browser() }
    LifecycleResumeEffect(controller) {
        val job = scope.launch { while (true) { controller.refresh(); delay(30_000) } }
        onPauseOrDispose { job.cancel(); controller.cancelConnection() }
    }
    AccountPage(state.copy(error = browserError ?: state.error), ::back, controller::signIn,
        controller::cancelSignIn, controller::signOut, controller::refresh, ::browser,
        { controller.connect(it, onOpen) }, controller::cancelConnection)
}

/** State-only layout also used by local review fixtures; never reads accounts or connects itself. */
@Composable
fun AccountPage(
    state: AccountUi, onBack: () -> Unit, onSignIn: () -> Unit, onCancelSignIn: () -> Unit,
    onSignOut: () -> Unit, onRefresh: () -> Unit, onBrowser: () -> Unit,
    onConnect: (OwnedComputer) -> Unit, onCancelConnection: () -> Unit,
) {
    val c = LocalMidnight.current
    Box(Modifier.fillMaxSize().background(c.bg).safeDrawingPadding(), contentAlignment = Alignment.TopCenter) {
        Column(Modifier.widthIn(max = 720.dp).fillMaxSize().padding(horizontal = 16.dp, vertical = 8.dp)) {
            Row(Modifier.fillMaxWidth().heightIn(min = 48.dp), verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back", tint = c.text) }
                SecparWordmark(Modifier.width(120.dp).height(30.dp))
                Spacer(Modifier.weight(1f))
                Text("Account", color = c.textDim, style = MaterialTheme.typography.titleSmall)
            }
            Spacer(Modifier.height(8.dp))
            if (state.user == null) {
                Text("Your computers", color = c.text, style = MaterialTheme.typography.titleLarge)
                Spacer(Modifier.height(8.dp))
                Text("Sign in to access your computers.", color = c.textDim, style = MaterialTheme.typography.bodyMedium)
                Spacer(Modifier.height(16.dp))
                val approval = state.approval
                if (approval != null) {
                    Text("Confirm this code in your browser", color = c.textDim, style = MaterialTheme.typography.bodyMedium)
                    Text(approval.userCode, color = c.text, style = MaterialTheme.typography.headlineMedium, fontFamily = FontFamily.Monospace,
                        modifier = Modifier.padding(vertical = 12.dp))
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(onClick = onBrowser) { Text("Open browser") }
                        OutlinedButton(onClick = onCancelSignIn) { Text("Cancel") }
                    }
                } else Button(onClick = onSignIn, enabled = state.ready && !state.busy) {
                    Text(if (state.busy) "Signing in…" else "Sign in")
                }
            } else {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    Text(state.user.handle, color = c.text, style = MaterialTheme.typography.titleMedium,
                            modifier = Modifier.weight(1f),
                            maxLines = 1, overflow = TextOverflow.Ellipsis)
                    TextButton(onClick = onRefresh, enabled = state.connecting == null) { Text("Refresh") }
                    TextButton(onClick = onSignOut) { Text("Sign out") }
                }
                HorizontalDivider(Modifier.padding(vertical = 4.dp), color = c.border)
                if (state.computers.isEmpty()) Text("No computers linked.", color = c.textDim,
                    style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(vertical = 12.dp))
                LazyColumn(Modifier.weight(1f)) {
                    items(state.computers, key = { it.id }) { computer ->
                        Row(Modifier.fillMaxWidth().heightIn(min = 60.dp), verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            Icon(Icons.Rounded.Computer, null, tint = c.accent, modifier = Modifier.size(24.dp))
                            Column(Modifier.weight(1f)) {
                                Text(computer.name, color = c.text, style = MaterialTheme.typography.bodyLarge,
                                    maxLines = 1, overflow = TextOverflow.Ellipsis)
                                val platform = when (computer.platform) { "windows" -> "Windows"; "linux" -> "Linux"; "macos" -> "macOS"; else -> computer.platform }
                                Text(if (!computer.linked) "Host update required" else "$platform · ${if (computer.online) "Online" else "Offline"}",
                                    color = c.textDim, style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            }
                            if (state.connecting == computer.id) OutlinedButton(onClick = onCancelConnection) { Text("Cancel") }
                            else OutlinedButton(onClick = { onConnect(computer) }, enabled = computer.linked && state.connecting == null) { Text("Connect") }
                        }
                        HorizontalDivider(color = c.border)
                    }
                }
            }
            state.error?.let { Text(it, color = c.bad, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(vertical = 8.dp)) }
        }
    }
}
