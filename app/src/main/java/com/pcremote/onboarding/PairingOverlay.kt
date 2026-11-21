package com.pcremote.onboarding

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.pcremote.R
import com.pcremote.ui.components.*
import com.pcremote.ui.theme.LocalMidnight

/** Progress and result while a pcremote://pair link (opened from a camera app or pasted) pairs. */
@Composable
fun PairingOverlay(state: PairingState, onRetry: () -> Unit, onDismiss: () -> Unit) {
    val c = LocalMidnight.current
    AnimatedContent(state, transitionSpec = { (fadeIn() + scaleIn(initialScale = 0.96f)) togetherWith fadeOut() }, label = "pair") { s ->
        when (s) {
            is PairingState.Pairing, is PairingState.Error -> Box(
                Modifier.fillMaxSize().background(c.bg.copy(alpha = 0.88f)).safeDrawingPadding().padding(20.dp),
                contentAlignment = Alignment.Center,
            ) {
                GlassCard(Modifier.fillMaxWidth(), padding = 16.dp) {
                    if (s is PairingState.Pairing) {
                        LinearProgressIndicator(Modifier.fillMaxWidth(), color = c.accent, trackColor = c.surfaceHigh)
                        Spacer(Modifier.height(20.dp))
                        Text(stringResource(R.string.pcr_ob_pairing, s.pcName), style = MaterialTheme.typography.titleMedium, color = c.text)
                    } else if (s is PairingState.Error) {
                        Icon(painterResource(R.drawable.lucide_circle_alert), null, tint = c.bad, modifier = Modifier.size(36.dp))
                        Spacer(Modifier.height(12.dp))
                        Text(stringResource(s.message), style = MaterialTheme.typography.bodyLarge, color = c.text)
                        Spacer(Modifier.height(20.dp))
                        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(0.dp)) {
                            PrimaryButton(stringResource(R.string.pcr_retry), onRetry)
                            GhostButton(stringResource(R.string.pcr_close), onDismiss, compact = true)
                        }
                    }
                }
            }
            else -> Spacer(Modifier)
        }
    }
}
