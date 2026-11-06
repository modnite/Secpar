package com.pcremote.update

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.ui.Alignment
import com.pcremote.settings.SettingsModel
import com.pcremote.settings.ThemeChoice
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.pcremote.BuildConfig
import com.pcremote.ui.theme.MidnightTheme

/** A separate route keeps installer prompts out of the streaming surface. */
class UpdatesActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            val settings = remember { SettingsModel.get(this@UpdatesActivity) }
            val dark = when (settings.theme) {
                ThemeChoice.DARK -> true
                ThemeChoice.LIGHT -> false
                ThemeChoice.SYSTEM -> isSystemInDarkTheme()
            }
            MidnightTheme(dark = dark, dynamic = settings.dynamicColor) {
                val state by Updater.state.collectAsState()
                val confirmation by Updater.confirmation.collectAsState()
                LaunchedEffect(confirmation) {
                    confirmation?.let { startActivity(it); Updater.clearConfirmation() }
                }
                Surface(Modifier.fillMaxSize()) {
                    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.TopCenter) {
                    Column(Modifier.widthIn(max = 520.dp).fillMaxWidth().safeDrawingPadding().padding(24.dp)
                        .verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                        Text("Updates", style = MaterialTheme.typography.headlineSmall)
                        Text("Installed: ${BuildConfig.VERSION_NAME}", style = MaterialTheme.typography.bodyMedium)
                        Text(state.message)
                        if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
                        if (state.release != null) {
                            Text("Installing restarts Secpar and ends any active stream.", style = MaterialTheme.typography.bodyMedium)
                            Button(onClick = { Updater.downloadAndInstall(this@UpdatesActivity) }, enabled = !state.busy) {
                                Text("Download & install")
                            }
                        }
                        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            OutlinedButton(onClick = { Updater.checkNow(this@UpdatesActivity) }, enabled = !state.busy) { Text("Check again") }
                            TextButton(onClick = { finish() }) { Text("Close") }
                        }
                    }
                    }
                }
            }
        }
        Updater.checkNow(this)
    }
}
