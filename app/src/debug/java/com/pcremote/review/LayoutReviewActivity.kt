package com.pcremote.review

import android.content.pm.ActivityInfo
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.Text
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.pcremote.home.*
import com.pcremote.nav.Mode
import com.pcremote.ui.theme.*

/** Debug-only layout fixture. Never opens a network connection or reads pairings. */
class LayoutReviewActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        requestedOrientation = if (intent.getBooleanExtra("wide", false)) ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE else ActivityInfo.SCREEN_ORIENTATION_PORTRAIT
        setContent {
            MidnightTheme(dark = !intent.getBooleanExtra("light", false)) {
                Box(Modifier.fillMaxSize()) {
                    HomeScreen(if (intent.getBooleanExtra("empty", false)) emptyList() else listOf(PcCardUi("fixture", "Studio PC", true, false, Mode.entries.toList(), PrimaryAction.OPEN, os = "Windows 11")), emptyMap(), Mode.entries.toSet(),
                        HomeActions(open = { _, _ -> }, primary = {}, addPc = {}, settings = {}, refresh = {}, rename = { _, _ -> }, remove = {}))
                    Text("Layout sample", color = LocalMidnight.current.textDim,
                        modifier = Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(bottom = 8.dp).background(LocalMidnight.current.bg))
                }
            }
        }
    }
}
