package com.pcremote.scan

import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.pcremote.update.UpdatesActivity
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.Assert.*
import java.io.File
import kotlinx.coroutines.flow.first

@RunWith(AndroidJUnit4::class)
class UpdatesInstrumentedTest {
    @Test fun updateScreenStartsAndFits() {
        ActivityScenario.launch(UpdatesActivity::class.java).use { scenario ->
            val instrumentation = InstrumentationRegistry.getInstrumentation()
            instrumentation.waitForIdleSync()
            scenario.onActivity { activity ->
                assertTrue(activity.window.decorView.isShown)
                assertFalse(activity.isFinishing)
            }
            kotlinx.coroutines.runBlocking {
                kotlinx.coroutines.withTimeoutOrNull(25_000) {
                    com.pcremote.update.Updater.state.first { !it.busy }
                }
            }
            instrumentation.waitForIdleSync()
            val bitmap = instrumentation.uiAutomation.takeScreenshot()
            assertNotNull(bitmap)
            File(instrumentation.targetContext.getExternalFilesDir(null), "updates-review.png").outputStream().use {
                bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it)
            }
            bitmap.recycle()
        }
    }
}
