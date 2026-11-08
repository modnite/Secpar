package com.pcremote.home

import android.graphics.Bitmap
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.requiredHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.background
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import com.pcremote.MainActivity
import com.pcremote.account.*
import com.pcremote.nav.Mode
import com.pcremote.onboarding.ManualPairDialog
import com.pcremote.onboarding.PairingOverlay
import com.pcremote.onboarding.PairingState
import com.pcremote.settings.SettingsModel
import com.pcremote.settings.SettingsPage
import com.pcremote.stream2.Failure
import com.pcremote.stream2.StreamUi
import com.pcremote.ui.theme.MidnightTheme
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** Synthetic render review for Home tabs, pairing, failure. No accounts, network or paired-device data. */
class ParityRenderInstrumentedTest {
    private val owner = "11111111-1111-4111-8111-111111111111"
    private val fr = "22222222-2222-4222-8222-"
    private fun card(id: String, name: String, online: Boolean, primary: PrimaryAction, os: String = "Windows 11",
        locked: Boolean = false, streaming: Boolean = false) =
        PcCardUi(id, name, online, streaming, if (online) Mode.entries.toList() else emptyList(), primary, canPower = online,
            locked = locked, canUnlock = locked, os = os, fingerprint = id)
    private val cards = listOf(
        card("a", "Studio computer", true, PrimaryAction.OPEN),
        card("b", "Travel laptop with a very long name that must truncate", false, PrimaryAction.WAKE, os = "Linux"),
        card("c", "Mac mini", true, PrimaryAction.OPEN, os = "macOS", locked = true),
        card("d", "Office", false, PrimaryAction.CHECKING),
    )
    private fun thumb(): ImageBitmap {
        val bmp = Bitmap.createBitmap(160, 90, Bitmap.Config.ARGB_8888)
        val canvas = android.graphics.Canvas(bmp)
        canvas.drawColor(android.graphics.Color.rgb(24, 40, 70))
        val p = android.graphics.Paint().apply { color = android.graphics.Color.rgb(200, 210, 230) }
        canvas.drawRect(16f, 14f, 100f, 28f, p); canvas.drawRect(16f, 40f, 140f, 76f, p)
        return bmp.asImageBitmap()
    }
    private val user = AccountUser(owner, "review-user")
    private val account = AccountUi(ready = true, user = user, computers = listOf(
        OwnedComputer(fr + "000000000001", "Cloud desktop", "windows", "b".repeat(64), true, true),
        OwnedComputer(fr + "000000000002", "Old host", "linux", null, false, false)))
    private val friends = AccountUi(ready = true, user = user, friends = listOf(
        Friend(fr + "000000000011", "river", "accepted", owner),
        Friend(fr + "000000000012", "sam", "pending", fr + "000000000012"),
        Friend(fr + "000000000013", "tess", "pending", owner)),
        blocked = listOf(AccountUser(fr + "000000000021", "noisy-neighbour")), note = AccountText.REQUEST_SENT)

    private val noop = HomeActions({ _, _ -> }, {}, {}, {}, { _, _ -> }, {})
    private fun home(tab: HomeTab, cards: List<PcCardUi>, acct: AccountUi, update: Boolean = false): @Composable () -> Unit = {
        val acts = AccountActions()
        val ctx = androidx.compose.ui.platform.LocalContext.current
        HomeScreen(cards, cards.associate { it.id to thumb() }.filterKeys { it == "a" }, Mode.entries.toSet(), noop, acct, acts, update, tab,
            friends = { FriendsPage(acct, acts) }, settings = { SettingsPage(SettingsModel.get(ctx), {}) })
    }

    @Test fun renderParityScreens() {
        val cases = linkedMapOf<String, @Composable () -> Unit>(
            "computers" to home(HomeTab.COMPUTERS, cards, AccountUi(ready = true)),
            "computers-signedin" to home(HomeTab.COMPUTERS, cards.take(2), account.copy(error = "Could not reach Secpar. Check your connection and try again."), update = true),
            "computers-empty" to home(HomeTab.COMPUTERS, emptyList(), AccountUi(ready = true)),
            "computers-approval" to home(HomeTab.COMPUTERS, emptyList(), AccountUi(ready = true, busy = true,
                approval = DeviceSignIn("d".repeat(64), "ABCD2345", "https://accounts.secpar.win/activate?code=ABCD2345", 600, 5))),
            "friends-signedout" to home(HomeTab.FRIENDS, cards, AccountUi(ready = true)),
            "friends" to home(HomeTab.FRIENDS, cards, friends),
            "settings-top" to home(HomeTab.SETTINGS, cards, AccountUi(ready = true)),
            "pair-dialog" to { Box(Modifier.background(Color(0xFF11151C))) { ManualPairDialog({}, {}) } },
            "pair-error" to { Box(Modifier.background(Color(0xFF11151C))) {
                PairingOverlay(PairingState.Error(com.pcremote.R.string.pcr_pair_failed), {}, {}) } },
            "pairing" to { Box(Modifier.background(Color(0xFF11151C))) { PairingOverlay(PairingState.Pairing("Studio computer"), {}, {}) } },
            "failure" to { Box(Modifier.background(Color(0xFF000000))) { Failure(StreamUi.Failed("capture_failed", ""), {}, {}) } },
        )
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            for ((name, content) in cases) capture(scenario, instrumentation, name, content)
            // Settings below the fold: slide a tall page up inside a clipped window.
            for (offset in listOf(40, 150, 330)) {
                capture(scenario, instrumentation, "settings-$offset", content = {
                    Box(Modifier.background(Color(0xFF11151C)).fillMaxSize().clipToBounds()) {
                        Box(Modifier.requiredHeight(1800.dp).offset(y = (-offset).dp)) {
                            SettingsPage(SettingsModel.get(androidx.compose.ui.platform.LocalContext.current), {})
                        }
                    }
                })
            }
            // The per-computer menu, opened through the accessibility tree.
            capture(scenario, instrumentation, "computers-menu", home(HomeTab.COMPUTERS, cards, AccountUi(ready = true))) {
                clickDescription(instrumentation, "More")
            }
        }
    }

    private fun clickDescription(instrumentation: android.app.Instrumentation, description: String) {
        val root = instrumentation.uiAutomation.rootInActiveWindow ?: return
        fun find(n: android.view.accessibility.AccessibilityNodeInfo): android.view.accessibility.AccessibilityNodeInfo? {
            if (n.contentDescription?.toString() == description) return n
            for (i in 0 until n.childCount) n.getChild(i)?.let { find(it)?.let { f -> return f } }
            return null
        }
        var node = find(root)
        while (node != null && !node.isClickable) node = node.parent
        node?.performAction(android.view.accessibility.AccessibilityNodeInfo.ACTION_CLICK)
    }

    private fun capture(scenario: ActivityScenario<MainActivity>, instrumentation: android.app.Instrumentation, name: String,
        content: @Composable () -> Unit, after: () -> Unit = {}) {
        val tag = InstrumentationRegistry.getArguments().getString("tag") ?: "d"
        val fileName = "parity-$name-$tag"

        val drawn = CountDownLatch(1)
        scenario.onActivity { activity ->
            activity.setContent {
                MidnightTheme(dark = true) {
                    Box(Modifier.fillMaxSize().drawWithContent { drawContent(); drawn.countDown() }) { content() }
                }
            }
        }
        assertTrue("$fileName did not draw", drawn.await(3, TimeUnit.SECONDS))
        settle(scenario, instrumentation)
        after()
        settle(scenario, instrumentation)
        val screen = instrumentation.uiAutomation.takeScreenshot()
        File(instrumentation.targetContext.filesDir, "$fileName.png").outputStream().use {
            assertTrue(screen.compress(Bitmap.CompressFormat.PNG, 100, it))
        }
    }

    private fun settle(scenario: ActivityScenario<MainActivity>, instrumentation: android.app.Instrumentation) {
        val presented = CountDownLatch(1)
        scenario.onActivity { a -> a.window.decorView.postOnAnimation { a.window.decorView.postOnAnimation { presented.countDown() } } }
        assertTrue(presented.await(3, TimeUnit.SECONDS))
        instrumentation.waitForIdleSync()
        Thread.sleep(250)
    }
}
