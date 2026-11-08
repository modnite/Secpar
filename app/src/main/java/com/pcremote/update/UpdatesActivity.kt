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
import androidx.compose.foundation.background
import androidx.compose.material3.*
import androidx.compose.ui.res.stringResource
import com.pcremote.R
import com.pcremote.ui.components.GhostButton
import com.pcremote.ui.components.PrimaryButton
import com.pcremote.ui.theme.LocalMidnight
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.pcremote.BuildConfig
import com.pcremote.ui.theme.MidnightTheme

const val UPDATE_WARNING = "Installing restarts Secpar and ends any active stream."

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
            SideEffect {
                androidx.core.view.WindowCompat.getInsetsController(window, window.decorView).apply {
                    isAppearanceLightStatusBars = !dark
                    isAppearanceLightNavigationBars = !dark
                }
            }
            MidnightTheme(dark = dark) {
                val state by Updater.state.collectAsState()
                val confirmation by Updater.confirmation.collectAsState()
                LaunchedEffect(confirmation) {
                    confirmation?.let { startActivity(it); Updater.clearConfirmation() }
                }
                val c = LocalMidnight.current
                Box(Modifier.fillMaxSize().background(c.bg), contentAlignment = Alignment.TopCenter) {
                    Column(Modifier.widthIn(max = 520.dp).fillMaxWidth().safeDrawingPadding().padding(20.dp)
                        .verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(stringResource(R.string.pcr_update_title), color = c.text, style = MaterialTheme.typography.headlineMedium)
                        Text(stringResource(R.string.pcr_updates_installed, BuildConfig.VERSION_NAME), color = c.textDim, style = MaterialTheme.typography.bodyMedium)
                        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            if (state.busy) CircularProgressIndicator(Modifier.size(16.dp), color = c.accent, strokeWidth = 2.dp)
                            Text(state.message, color = c.text, style = MaterialTheme.typography.bodyMedium)
                        }
                        if (state.release != null) {
                            Text(UPDATE_WARNING, color = c.textDim, style = MaterialTheme.typography.bodySmall)
                        }
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            GhostButton(stringResource(R.string.pcr_update_check), { Updater.checkNow(this@UpdatesActivity) }, enabled = !state.busy)
                            PrimaryButton(stringResource(R.string.pcr_updates_download), { Updater.downloadAndInstall(this@UpdatesActivity) },
                                enabled = state.release != null && !state.busy)
                        }
                    }
                }
            }
        }
        Updater.checkNow(this)
    }
}
