package com.pcremote.nav

import android.content.pm.ActivityInfo
import android.graphics.Rect
import androidx.activity.compose.setContent
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.lifecycle.Lifecycle
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import com.pcremote.MainActivity
import com.pcremote.home.*
import com.pcremote.ui.theme.MidnightTheme
import org.junit.Assert.*
import org.junit.Test

/** Synthetic window-inset fault; no accounts, real thumbnails, or stream connection. */
class HomeInsetsInstrumentedTest {
    @Test fun nonStreamResumeRestoresLingeringImmersiveBars() {
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            var focused = false
            waitUntil { scenario.onActivity { focused = it.hasWindowFocus() }; focused }
            val settled = java.util.concurrent.CountDownLatch(1)
            scenario.onActivity { activity -> activity.window.decorView.postOnAnimation {
                activity.window.decorView.postOnAnimation { settled.countDown() }
            } }
            assertTrue(settled.await(3, java.util.concurrent.TimeUnit.SECONDS))
            scenario.onActivity { activity ->
                androidx.core.view.WindowCompat.getInsetsController(activity.window, activity.window.decorView)
                    .hide(WindowInsetsCompat.Type.systemBars())
            }
            var hidden = false
            waitUntil {
                scenario.onActivity { hidden = ViewCompat.getRootWindowInsets(it.window.decorView)
                    ?.isVisible(WindowInsetsCompat.Type.statusBars()) == false }
                hidden
            }
            scenario.moveToState(Lifecycle.State.CREATED)
            scenario.moveToState(Lifecycle.State.RESUMED)
            var shown = false
            waitUntil {
                scenario.onActivity { shown = ViewCompat.getRootWindowInsets(it.window.decorView)
                    ?.isVisible(WindowInsetsCompat.Type.statusBars()) == true }
                shown
            }
        }
    }
    @Test fun returningHomeDiscardsStaleLandscapeInsetsWithoutRecreation() {
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            scenario.onActivity { activity ->
                activity.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_PORTRAIT
                activity.setContent {
                    MidnightTheme(dark = true, dynamic = false) {
                        HomeScreen(listOf(PcCardUi("fixture", "Review PC", true, false,
                            Mode.entries, PrimaryAction.OPEN)), emptyMap(), Mode.entries.toSet(),
                            HomeActions({ _, _ -> }, {}, {}, {}, {}, { _, _ -> }, {}))
                    }
                }
            }
            waitUntil { accountBounds() != null }
            val initial = accountBounds()!!
            scenario.moveToState(Lifecycle.State.CREATED)
            scenario.moveToState(Lifecycle.State.RESUMED)
            waitUntil { accountBounds() == initial }
            // Reproduce the screenshot's left inset / missing top inset. Dispatch
            // directly into the retained Compose view, like a stale resize event.
            scenario.onActivity { activity ->
                val content = activity.findViewById<android.view.ViewGroup>(android.R.id.content)
                val compose = content.getChildAt(0)
                val current = ViewCompat.getRootWindowInsets(compose)!!
                val stale = WindowInsetsCompat.Builder(current)
                    .setInsets(WindowInsetsCompat.Type.statusBars(), Insets.of(96, 0, 0, 0))
                    .setInsetsIgnoringVisibility(WindowInsetsCompat.Type.statusBars(), Insets.of(96, 0, 0, 0))
                    .setVisible(WindowInsetsCompat.Type.statusBars(), true).build()
                compose.dispatchApplyWindowInsets(stale.toWindowInsets()!!)
            }
            waitUntil { accountBounds()?.let { it != initial } == true }
            scenario.onActivity { it.onWindowFocusChanged(true) }
            waitUntil { accountBounds() == initial }
            scenario.onActivity { activity ->
                val insets = ViewCompat.getRootWindowInsets(activity.window.decorView)!!
                assertTrue("Home must restore system bars", insets.isVisible(WindowInsetsCompat.Type.statusBars()))
            }
        }
    }

    private fun accountBounds(): Rect? {
        val root = InstrumentationRegistry.getInstrumentation().uiAutomation.rootInActiveWindow ?: return null
        fun find(node: android.view.accessibility.AccessibilityNodeInfo): Rect? {
            if (node.contentDescription?.toString() == "Account") return Rect().also { node.getBoundsInScreen(it) }
            for (i in 0 until node.childCount) node.getChild(i)?.let { find(it)?.let { rect -> return rect } }
            return null
        }
        return find(root)
    }

    private fun waitUntil(condition: () -> Boolean) {
        val deadline = System.currentTimeMillis() + 6000
        while (!condition() && System.currentTimeMillis() < deadline) Thread.sleep(50)
        assertTrue("Home inset bounds did not settle", condition())
    }
}
