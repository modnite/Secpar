package com.pcremote.modes

import android.os.Build
import android.view.WindowManager
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.pcremote.R
import com.pcremote.agent.AgentConnection
import com.pcremote.agent.AgentState
import com.pcremote.agent.FailReason
import com.pcremote.agent.Msg
import com.pcremote.agent.PairedPc
import com.pcremote.agent.PcStore
import com.pcremote.ui.components.Aurora
import com.pcremote.ui.components.GhostButton
import com.pcremote.ui.components.GlassIconButton
import com.pcremote.ui.components.StatusDot
import com.pcremote.ui.components.glass
import com.pcremote.ui.theme.LocalMidnight
import com.pcremote.ui.theme.MidnightShapes
import kotlinx.coroutines.launch

/** A live agent connection owned by one mode screen. */
class AgentSession(val pc: PairedPc, val conn: AgentConnection) {
    /** Checks the live connection state, so callbacks captured early still work once online. */
    fun send(m: Msg) = conn.sendIfOnline(m)
}

/** Opens an agent connection for [pcId] while the calling screen is on screen. */
@Composable
fun rememberAgentSession(pcId: String): AgentSession? {
    if (com.pcremote.guest.GuestSessions.isGuest(pcId)) return rememberGuestSession(pcId)
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    val accountRevision by com.pcremote.account.AccountRuntime.get(ctx).controller.revision.collectAsState()
    val pc = remember(pcId, accountRevision) { PcStore.get(ctx).find(pcId) } ?: return null
    // Ordinary account refreshes must not restart an existing manual stream.
    val session = remember(pcId, pc.accountOwner, pc.token, pc.fingerprint) { AgentSession(pc, AgentConnection.forPc(pc, scope)) }
    DisposableEffect(session) {
        val hello = scope.launch { session.conn.hello(pc.token, Build.MODEL) }
        // PC → phone files are received in the background, whichever screen is open
        val incoming = scope.launch { session.conn.incoming.collect { if (it is Msg.FileOffer) com.pcremote.modes.TransferHub.get(ctx, pcId).offer(it) } }
        onDispose { hello.cancel(); incoming.cancel(); session.conn.close() }
    }
    return session
}

/** A friend's computer: the socket is already joined, and it is closed when the screen goes away. */
@Composable
private fun rememberGuestSession(route: String): AgentSession? {
    val link = remember(route) { com.pcremote.guest.GuestSessions.find(route) } ?: return null
    val session = remember(link) { AgentSession(link.pc, link.conn) }
    DisposableEffect(session) {
        onDispose { link.conn.close(); com.pcremote.guest.GuestSessions.remove(route) }
    }
    return session
}

/** Keeps the phone screen on while a mode is open. */
@Composable
fun KeepScreenOn() {
    val window = (LocalContext.current as? android.app.Activity)?.window ?: return
    DisposableEffect(window) {
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        onDispose { window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) }
    }
}

/** Shared chrome for every mode: back, PC name, live status, power, unlock, and a connection banner. */
@Composable
fun ModeScaffold(
    title: Int,
    session: AgentSession?,
    onBack: () -> Unit,
    showPower: Boolean = true,
    content: @Composable ColumnScope.(AgentState) -> Unit,
) {
    val c = LocalMidnight.current
    val scope = rememberCoroutineScope()
    val state by (session?.conn?.state ?: remember { kotlinx.coroutines.flow.MutableStateFlow(AgentState.Closed) }).collectAsState()
    val locked by (session?.conn?.locked ?: remember { kotlinx.coroutines.flow.MutableStateFlow(false) }).collectAsState()
    val unlock = rememberUnlocker(session?.pc?.id.orEmpty(), { m -> session?.send(m) == true }, session?.conn?.incoming)
    BackHandler(onBack = onBack)
    KeepScreenOn()
    Box(Modifier.fillMaxSize().background(c.bg)) {
        Aurora(intensity = 0.45f)
        Column(Modifier.fillMaxSize().safeDrawingPadding()) {
            Row(Modifier.fillMaxWidth().padding(16.dp, 10.dp, 16.dp, 10.dp), verticalAlignment = Alignment.CenterVertically) {
                GlassIconButton(R.drawable.lucide_arrow_left, stringResource(R.string.pcr_back), onBack)
                Spacer(Modifier.width(14.dp))
                Column(Modifier.weight(1f)) {
                    Text(stringResource(title), color = c.text, style = MaterialTheme.typography.titleLarge)
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        val online = state is AgentState.Online
                        StatusDot(if (locked && online) c.warn else if (online) c.ok else if (state is AgentState.Failed) c.bad else c.warn)
                        Text(session?.pc?.name ?: "", color = c.textDim, style = MaterialTheme.typography.bodySmall,
                            maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                }
                var power by remember { mutableStateOf(false) }
                if (power && session != null) PowerSheet(session.pc.name, { session.send(it) }) { power = false }
                if (showPower && state is AgentState.Online && "power" in (state as AgentState.Online).caps) {
                    GlassIconButton(R.drawable.lucide_power, stringResource(R.string.pcr_power), { power = true })
                }
            }
            val online = state as? AgentState.Online
            AnimatedVisibility(online != null && locked && "unlock" in online.caps) {
                Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp).glass(MidnightShapes.control).padding(start = 16.dp, end = 8.dp, top = 8.dp, bottom = 8.dp),
                    verticalAlignment = Alignment.CenterVertically) {
                    Icon(painterResource(R.drawable.lucide_lock), null, tint = c.warn, modifier = Modifier.size(18.dp))
                    Spacer(Modifier.width(10.dp))
                    Text(stringResource(R.string.pcr_status_locked), color = c.text, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
                    GhostButton(stringResource(R.string.pcr_unlock), unlock, icon = R.drawable.lucide_fingerprint)
                }
            }
            AnimatedVisibility(state !is AgentState.Online) {
                val msg = when ((state as? AgentState.Failed)?.reason) {
                    null -> R.string.pcr_conn_connecting
                    FailReason.REVOKED, FailReason.UNAUTHORIZED -> R.string.pcr_conn_revoked
                    FailReason.CERT_MISMATCH -> R.string.pcr_cert_mismatch
                    else -> R.string.pcr_conn_unreachable
                }
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp).glass(MidnightShapes.control)
                        .padding(horizontal = 16.dp, vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(stringResource(msg), color = c.text, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
                    if (state is AgentState.Failed && session != null) {
                        GhostButton(stringResource(R.string.pcr_retry), {
                            scope.launch { session.conn.hello(session.pc.token, Build.MODEL) }
                        })
                    }
                }
            }
            if (session == null) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Text(stringResource(R.string.pcr_conn_unknown_pc), color = c.textDim)
                }
            } else {
                ClipboardSync(session, state)
                content(state)
            }
        }
    }
}
