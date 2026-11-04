package com.pcremote.stream2

import android.view.KeyEvent
import android.view.MotionEvent
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.pcremote.MainActivity
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class CaptureRoutingInstrumentedTest {
    @Test fun releasedPointerBypassesRemoteHandlersAndShortcutTogglesOnlyOnce() {
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            scenario.onActivity { activity ->
                val oldCapture = activity.desktopCapture
                val oldMotion = activity.desktopMotion
                var remoteMouseEvents = 0
                val capture = DesktopCapture({}, {})
                activity.desktopCapture = capture
                activity.desktopMotion = { remoteMouseEvents++; true }
                try {
                    capture.setRequested(true)
                    val motion = MotionEvent.obtain(0, 1, MotionEvent.ACTION_HOVER_MOVE, 50f, 50f, 0)
                    motion.source = android.view.InputDevice.SOURCE_MOUSE
                    activity.dispatchGenericMotionEvent(motion)
                    assertEquals(1, remoteMouseEvents)
                    activity.dispatchKeyEvent(KeyEvent(0, 1, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_F12, 0, KeyEvent.META_CTRL_ON))
                    assertFalse(capture.forwarding)
                    activity.dispatchKeyEvent(KeyEvent(0, 2, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_F12, 1, KeyEvent.META_CTRL_ON))
                    activity.dispatchKeyEvent(KeyEvent(0, 3, KeyEvent.ACTION_UP, KeyEvent.KEYCODE_F12, 0, 0))
                    assertFalse(capture.forwarding)
                    activity.dispatchGenericMotionEvent(motion)
                    val click = MotionEvent.obtain(0, 4, MotionEvent.ACTION_DOWN, 50f, 50f, 0)
                    click.source = android.view.InputDevice.SOURCE_MOUSE
                    activity.dispatchTouchEvent(click)
                    assertEquals("Released mouse must never reach the remote handler", 1, remoteMouseEvents)
                    click.recycle(); motion.recycle()
                    activity.dispatchKeyEvent(KeyEvent(0, 5, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_F12, 0, KeyEvent.META_CTRL_ON))
                    assertTrue(capture.forwarding)
                    activity.dispatchKeyEvent(KeyEvent(0, 6, KeyEvent.ACTION_UP, KeyEvent.KEYCODE_F12, 0, 0))
                } finally {
                    activity.desktopCapture = oldCapture
                    activity.desktopMotion = oldMotion
                }
            }
        }
    }

    @Test fun releasedMouseCanClickAnAndroidControl() {
        var clicked = false
        lateinit var button: android.widget.Button
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            scenario.onActivity { activity ->
                button = android.widget.Button(activity).apply {
                    text = "Local Android control"
                    setOnClickListener { clicked = true }
                }
                activity.setContentView(button)
                activity.desktopCapture = DesktopCapture({}, {})
                activity.desktopMotion = { throw AssertionError("Released mouse was intercepted") }
            }
            androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().waitForIdleSync()
            scenario.onActivity { activity ->
                val location = IntArray(2)
                button.getLocationInWindow(location)
                val point = MotionEvent.PointerCoords().apply {
                    x = location[0] + button.width / 2f
                    y = location[1] + button.height / 2f
                }
                val props = MotionEvent.PointerProperties().apply { id = 0; toolType = MotionEvent.TOOL_TYPE_MOUSE }
                val now = android.os.SystemClock.uptimeMillis()
                for (action in listOf(MotionEvent.ACTION_DOWN, MotionEvent.ACTION_UP)) {
                    val event = MotionEvent.obtain(now, now + action, action, 1, arrayOf(props), arrayOf(point),
                        0, if (action == MotionEvent.ACTION_DOWN) MotionEvent.BUTTON_PRIMARY else 0,
                        1f, 1f, 0, 0, android.view.InputDevice.SOURCE_MOUSE, 0)
                    activity.dispatchTouchEvent(event)
                    event.recycle()
                }
                activity.desktopCapture = null
                activity.desktopMotion = null
            }
            androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().waitForIdleSync()
            assertTrue("Android UI must receive the released mouse click", clicked)
        }
    }
}
