package com.pcremote.guest

import android.widget.Toast
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.platform.LocalContext
import com.pcremote.MainActivity
import com.pcremote.agent.AgentState
import com.pcremote.agent.Msg
import com.pcremote.modes.AgentSession
import kotlinx.coroutines.launch

/**
 * Stream screen parts that only apply to a friend's computer: the host's error frames, the end of the
 * session, and the controllers plugged into this device. Does nothing for a paired computer.
 */
@Composable
fun GuestSessionEffects(session: AgentSession, onEnded: () -> Unit) {
    if (!GuestSessions.isGuest(session.pc.id)) return
    val ctx = LocalContext.current
    val end by rememberUpdatedState(onEnded)
    LaunchedEffect(session) {
        val shown = HashSet<String>()
        session.conn.incoming.collect { m ->
            if (m is Msg.Error) {
                if (shown.add(m.code) || GuestMessages.ends(m.code)) Toast.makeText(ctx, GuestMessages.forCode(m.code, m.msg), Toast.LENGTH_LONG).show()
                if (GuestMessages.ends(m.code)) end()
            }
        }
    }
    val state by session.conn.state.collectAsState()
    LaunchedEffect(state) {
        if (state is AgentState.Closed || state is AgentState.Failed) {
            Toast.makeText(ctx, "The connection ended.", Toast.LENGTH_LONG).show()
            end()
        }
    }
    GuestControllers(session)
}

@Composable
private fun GuestControllers(session: AgentSession) {
    val state by session.conn.state.collectAsState()
    // The welcome lists `pad` only when the grant allows controllers.
    val allowed = (state as? AgentState.Online)?.caps?.contains("pad") == true
    com.pcremote.modes.PhysicalPadsEffect(session::send, session.conn.incoming, allowed)
}
