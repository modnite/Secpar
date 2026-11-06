package com.pcremote.stream2

import android.content.Intent
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import com.pcremote.MainActivity
import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.atomic.AtomicInteger

class PictureInPictureInstrumentedTest {
    @Test fun pipReleasesInputAndReturnsWithoutRecreatingTheVideoSurface() {
        val created = AtomicInteger()
        var activityRef: MainActivity? = null
        var capture: DesktopCapture? = null
        var hidden = false
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            scenario.onActivity { activity ->
                assertTrue("Review device must support PiP", activity.supportsPip)
                activityRef = activity
                activity.onStreamHidden = { hidden = true }
                activity.setContentView(SurfaceView(activity).apply {
                    holder.addCallback(object : SurfaceHolder.Callback {
                        override fun surfaceCreated(holder: SurfaceHolder) { created.incrementAndGet() }
                        override fun surfaceChanged(holder: SurfaceHolder, format: Int, w: Int, h: Int) {}
                        override fun surfaceDestroyed(holder: SurfaceHolder) {}
                    })
                })
                capture = DesktopCapture({}, {}).also { it.setRequested(true) }
                activity.desktopCapture = capture
                activity.configureStreamPip(true, 1920, 1080)
            }
            waitUntil { created.get() == 1 }
            scenario.onActivity { assertTrue(it.enterStreamPip()) }
            waitUntil { activityRef?.pipVisible == true }
            assertFalse("PiP must release all remote input", capture!!.forwarding)
            val instrumentation = InstrumentationRegistry.getInstrumentation()
            instrumentation.targetContext.startActivity(Intent(instrumentation.targetContext, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_REORDER_TO_FRONT))
            waitUntil { activityRef?.pipVisible == false }
            scenario.onActivity {
                assertSame("Activity must survive PiP resize", activityRef, it)
                assertEquals("Video surface must survive PiP entry/exit", 1, created.get())
                assertFalse("PiP must keep the active stream alive", hidden)
            }
            // Finish the resumed task after disabling auto-entry, avoiding ActivityScenario's
            // synthetic background activity which races the system's PiP transition.
            scenario.onActivity {
                it.configureStreamPip(false)
                assertFalse("PiP must be disabled outside a stream", it.enterStreamPip())
                it.desktopCapture = null
            }

        }
    }
    @Test fun dismissingPipEndsTheStream() {
        var hidden = false
        var activityRef: MainActivity? = null
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            scenario.onActivity {
                activityRef = it
                it.onStreamHidden = { hidden = true }
                it.configureStreamPip(true)
            }
            waitUntil { activityRef?.hasWindowFocus() == true }
            scenario.onActivity { assertTrue(it.enterStreamPip()) }
            waitUntil { activityRef?.pipVisible == true }
            scenario.onActivity { it.finishAndRemoveTask() }
            waitUntil { hidden }
        }
    }
    private fun waitUntil(condition: () -> Boolean) {
        val deadline = System.currentTimeMillis() + 8000
        while (!condition() && System.currentTimeMillis() < deadline) Thread.sleep(50)
        assertTrue("Timed out waiting for Android PiP", condition())
    }
}
