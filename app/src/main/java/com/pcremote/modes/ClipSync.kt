package com.pcremote.modes

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalWindowInfo
import com.pcremote.agent.AgentState
import com.pcremote.agent.Msg
import kotlinx.coroutines.flow.onSubscription

/** Two-way text clipboard policy. Create a fresh instance for each online session. */
class ClipSync {
    private var last: String? = null
    private var seeded = false

    /** Seed the first readable clipboard without replacing the PC's preexisting clipboard. */
    fun onFocus(text: String?): Msg.Clip? {
        if (!seeded) {
            seeded = true
            if (text != null && fits(text)) last = text
            return null
        }
        return text?.let(::onLocalChange)
    }

    /** A focused local copy, or a changed clipboard observed after returning to the app. */
    fun onLocalChange(text: String): Msg.Clip? {
        if (text == last || !fits(text)) return null
        last = text
        return Msg.Clip("text/plain", text)
    }

    /** The PC sent its clipboard; returns the text to put on Android, if any. */
    fun onRemote(m: Msg.Clip): String? {
        if (m.mime != "text/plain" || m.data == last || !fits(m.data)) return null
        last = m.data
        return m.data
    }

    private fun fits(text: String) = text.length <= MAX_TEXT && text.toByteArray(Charsets.UTF_8).size <= MAX_TEXT

    companion object { const val MAX_TEXT = 1_000_000 }
}

/**
 * Android clipboard reads require window focus, including in multi-window/DeX. Listen only while
 * focused, and compare the clipboard on return to catch copies made in another app. The first
 * readable value is only a baseline, so opening/reconnecting never pushes stale Android text.
 */
@Composable
fun ClipboardSync(session: AgentSession?, state: AgentState) {
    val ctx = LocalContext.current
    val cm = remember(ctx) { ctx.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager }
    val online = session != null && state is AgentState.Online && "clip" in state.caps
    val sync = remember(session, online) { ClipSync() }
    val focused = LocalWindowInfo.current.isWindowFocused
    val currentFocus = rememberUpdatedState(focused)
    LaunchedEffect(session, online, sync) {
        if (!online || session == null) return@LaunchedEffect
        try {
            // Subscribe only once the receiver is listening, including on reconnect.
            session.conn.incoming.onSubscription { session.send(Msg.ClipSub(true)) }.collect { m ->
                if (m is Msg.Clip) sync.onRemote(m)?.let {
                    cm.setPrimaryClip(ClipData.newPlainText("PC", it))
                }
            }
        } finally {
            session.send(Msg.ClipSub(false))
        }
    }
    DisposableEffect(session, online, focused, sync, cm) {
        var active = online && focused
        fun readText(onText: (String?) -> Unit) {
            if (!active || !currentFocus.value) return
            val text = try {
                // Do not coerce URI clips: synchronization is limited to actual text.
                cm.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.text?.toString()
            } catch (_: SecurityException) {
                return // Focus can be revoked between the check and the platform read.
            }
            onText(text)
        }
        val listener = ClipboardManager.OnPrimaryClipChangedListener {
            readText { text -> text?.let { sync.onLocalChange(it)?.let { m -> session?.send(m) } } }
        }
        if (active) {
            readText { text -> sync.onFocus(text)?.let { session?.send(it) } }
            cm.addPrimaryClipChangedListener(listener)
        }
        onDispose {
            active = false
            cm.removePrimaryClipChangedListener(listener)
        }
    }
}
