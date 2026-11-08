package com.pcremote.onboarding

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.res.stringResource
import com.pcremote.R
import com.pcremote.agent.PairUri
import com.pcremote.ui.components.FlatDialog
import com.pcremote.ui.components.FlatField
import com.pcremote.ui.components.GhostButton
import com.pcremote.ui.components.PrimaryButton

/** For phones without a usable camera: paste the pairing link copied from the computer. */
@Composable
fun ManualPairDialog(onPair: (String) -> Unit, onDismiss: () -> Unit) {
    val clip = LocalClipboardManager.current
    var text by remember { mutableStateOf(clip.getText()?.text?.takeIf { PairUri.parse(it.trim()) != null }.orEmpty()) }
    val valid = PairUri.parse(text.trim()) != null
    FlatDialog(stringResource(R.string.pcr_manual_title), onDismiss, actions = {
        GhostButton(stringResource(R.string.pcr_cancel), onDismiss, compact = true)
        PrimaryButton(stringResource(R.string.pcr_pair), { onPair(text.trim()) }, enabled = valid)
    }) {
        FlatField(text, { text = it }, stringResource(R.string.pcr_manual_hint), Modifier.fillMaxWidth(), masked = true,
            isError = text.isNotBlank() && !valid)
    }
}
