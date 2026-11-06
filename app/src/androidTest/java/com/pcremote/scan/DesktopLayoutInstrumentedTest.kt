package com.pcremote.scan

import androidx.activity.compose.setContent
import androidx.compose.runtime.mutableStateOf
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import com.pcremote.MainActivity
import com.pcremote.home.*
import com.pcremote.nav.Mode
import com.pcremote.ui.theme.MidnightTheme
import org.junit.Assert.*
import org.junit.Test
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** Render-only review fixture: inspect saved images at actual wide/phone sizes.
 * No connection, real thumbnails, accounts or paired-device data. */
class DesktopLayoutInstrumentedTest {
    @Test fun renderCheckingAndConnectedReviewFixtures() {
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        val automation=instrumentation.uiAutomation
        val state=mutableStateOf(false)
        val args=InstrumentationRegistry.getArguments()
        val width=args.getString("review_width_dp")?.toIntOrNull() ?: 0
        val height=args.getString("review_height_dp")?.toIntOrNull() ?: 0
        ActivityScenario.launch(MainActivity::class.java).use {scenario ->
            scenario.onActivity {activity ->
                activity.setContent {
                    MidnightTheme(dark=true,dynamic=false) {
                        Box(if(width>0 && height>0)Modifier.size(width.dp,height.dp) else Modifier.fillMaxSize()) {
                        HomeScreen(listOf(PcCardUi("layout-fixture","Review PC",state.value,false,Mode.entries,
                            if(state.value)PrimaryAction.OPEN else PrimaryAction.CHECKING,os="Windows")),emptyMap(),Mode.entries.toSet(),
                            HomeActions({_,_->},{},{},{},{},{_,_->},{}))
                        }
                    }
                }
            }
            for(online in listOf(false,true)) {
                val drawn=CountDownLatch(1)
                scenario.onActivity {activity ->
                    state.value=online
                    activity.window.decorView.postOnAnimation {
                        activity.window.decorView.postOnAnimation {drawn.countDown()}
                    }
                }
                assertTrue("Review frame did not draw",drawn.await(3,TimeUnit.SECONDS))
                instrumentation.waitForIdleSync()
                val bitmap=automation.takeScreenshot()
                File(instrumentation.targetContext.filesDir,if(online)"home-connected.png" else "home-checking.png").outputStream().use {
                    assertTrue(bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it))
                }
            }
        }
    }
}
