package com.pcremote.account

import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import com.pcremote.MainActivity
import com.pcremote.ui.theme.MidnightTheme
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** Synthetic render review only; never signs in or reads account credentials. */
class AccountLayoutInstrumentedTest {
    @Test fun renderAccountAtDockAndPhoneSizes() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val cases = listOf(
            "signed-out" to AccountUi(ready = true),
            "computers" to AccountUi(ready = true, user = AccountUser("fixture", "review-user"), computers = listOf(
                OwnedComputer("fixture-a", "Studio PC", "windows", null, true, true),
                OwnedComputer("fixture-b", "Travel laptop", "linux", null, true, false),
                OwnedComputer("fixture-c", "Mac", "macos", null, false, false))),
        )
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            for ((width, height) in listOf(600 to 420, 360 to 640)) for ((name, state) in cases) {
                val drawn = CountDownLatch(1)
                scenario.onActivity { activity ->
                    activity.setContent {
                        MidnightTheme(dark = true, dynamic = false) {
                            Box(Modifier.size(width.dp, height.dp).drawWithContent { drawContent(); drawn.countDown() }) {
                                AccountPage(state, {}, {}, {}, {}, {}, {}, {}, {})
                            }
                        }
                    }
                }
                assertTrue("Account fixture did not draw", drawn.await(3, TimeUnit.SECONDS))
                val presented = CountDownLatch(1)
                scenario.onActivity { activity -> activity.window.decorView.postOnAnimation {
                    activity.window.decorView.postOnAnimation { presented.countDown() }
                } }
                assertTrue("Account frame did not present", presented.await(3, TimeUnit.SECONDS))
                instrumentation.waitForIdleSync()
                val screen = instrumentation.uiAutomation.takeScreenshot()
                val cropped = android.graphics.Bitmap.createBitmap(screen, 0, 0, width, height)
                File(instrumentation.targetContext.filesDir, "account-$name-${width}x$height.png").outputStream().use {
                    assertTrue(cropped.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it))
                }
            }
        }
    }
}
