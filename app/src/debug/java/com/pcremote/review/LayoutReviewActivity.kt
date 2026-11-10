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
                    if (intent.getBooleanExtra("controls", false)) {
                        Text("Stream layout sample", color = LocalMidnight.current.textDim,
                            modifier = Modifier.align(Alignment.Center))
                        com.pcremote.stream.StreamOverlay(com.pcremote.stream.ToolbarState().apply {
                            connected = true; canPip = true; pcName = "Studio computer"
                        }, object : com.pcremote.stream.ToolbarActions {
                            override fun shortcut(s: com.pcremote.stream.Shortcut) {}
                            override fun keyboard() {}
                            override fun setTrackpad(on: Boolean) {}
                            override fun disconnect() {}
                            override fun keys(events: List<com.pcremote.stream.KeyEvt>) {}
                            override fun selectDisplay(id: String) {}
                        })
                    } else if (intent.getBooleanExtra("friends", false)) {
                        val state = com.pcremote.account.AccountUi(ready = true,
                            user = com.pcremote.account.AccountUser("review", "Layout sample"),
                            friends = listOf(
                                com.pcremote.account.Friend("jamie", "Jamie", "accepted", "review", "full", "always"),
                                com.pcremote.account.Friend("riley", "Riley", "accepted", "review"),
                                com.pcremote.account.Friend("alex", "Alex", "pending", "alex")))
                        HomeScreen(emptyList(), emptyMap(), Mode.entries.toSet(),
                            HomeActions(open = { _, _ -> }, primary = {}, addPc = {}, refresh = {}, rename = { _, _ -> }, remove = {}),
                            account = state, initialTab = HomeTab.FRIENDS,
                            friends = { com.pcremote.account.FriendsPage(state, com.pcremote.account.AccountActions()) })
                    } else HomeScreen(if (intent.getBooleanExtra("empty", false)) emptyList() else buildList {
                            add(PcCardUi("fixture", "Studio PC", true, false, Mode.entries.toList(), PrimaryAction.OPEN, os = "Windows 11"))
                            if (intent.getBooleanExtra("multi", false)) add(PcCardUi("fixture-2", "Linux laptop", true, false, Mode.entries.toList(), PrimaryAction.OPEN, os = "KDE Plasma"))
                        }, emptyMap(), Mode.entries.toSet(),
                        HomeActions(open = { _, _ -> }, primary = {}, addPc = {}, refresh = {}, rename = { _, _ -> }, remove = {}),
                        account = if (intent.getBooleanExtra("multi", false)) com.pcremote.account.AccountUi(ready = true,
                            user = com.pcremote.account.AccountUser("review", "Layout sample")) else com.pcremote.account.AccountUi())
                    Text("Layout sample", color = LocalMidnight.current.textDim,
                        modifier = Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(bottom = 8.dp).background(LocalMidnight.current.bg))
                }
            }
        }
    }
}
