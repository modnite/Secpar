package com.pcremote.engine

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.activity.compose.BackHandler
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.pcremote.modes.AgentSession
import com.pcremote.ui.components.GhostButton
import com.pcremote.ui.components.PrimaryButton

/**
 * The one moment between tapping a computer and Moonlight's stream: ask the computer for its
 * engine, pair this device through the control connection, and open the tunnel. Moonlight's
 * own `Game` activity then draws video and takes input; the app gets this screen back when it
 * ends, and the tunnel is closed with it.
 */
@Composable
fun EngineStreamScreen(session: AgentSession, onBack: () -> Unit) {
    val context = LocalContext.current
    val settings = remember { com.pcremote.settings.SettingsModel.get(context) }
    var problem by remember { mutableStateOf<String?>(null) }
    var attempt by remember { mutableIntStateOf(0) }

    BackHandler(onBack = onBack)

    // The engine's activity is singleTask, so nothing comes back through a result. This app
    // coming to the foreground again means the stream has ended.
    var launched by remember { mutableStateOf(false) }
    val lifecycle = LocalLifecycleOwner.current
    DisposableEffect(lifecycle) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME && launched) {
                EngineLaunch.finish()
                onBack()
            }
        }
        lifecycle.lifecycle.addObserver(observer)
        onDispose { lifecycle.lifecycle.removeObserver(observer) }
    }

    LaunchedEffect(attempt) {
        problem = null
        val client = GamestreamClient(session::send, session.conn.incoming)
        EngineLaunch.prepare(context, client, session.pc.name, settings, session.conn.connectedHost)
            .onSuccess { intent ->
                launched = true
                EngineLaunch.launch(context, intent)
            }
            .onFailure { problem = it.message ?: "The stream did not start." }
    }

    // Leaving this screen before the stream started must not leave a tunnel socket behind.
    DisposableEffect(Unit) { onDispose { EngineLaunch.finish() } }

    Column(
        Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).padding(24.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(session.pc.name, style = MaterialTheme.typography.titleMedium)
        Spacer(Modifier.height(6.dp))
        Text(
            problem ?: "Starting the stream",
            style = MaterialTheme.typography.bodyMedium,
            color = if (problem == null) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.error,
            textAlign = TextAlign.Center,
        )
        if (problem != null) {
            Spacer(Modifier.height(16.dp))
            PrimaryButton("Try again", onClick = { attempt += 1 })
            Spacer(Modifier.height(8.dp))
            GhostButton("Back", onBack)
        }
    }
}
