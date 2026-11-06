package com.pcremote.scan

import android.graphics.Rect
import androidx.activity.compose.setContent
import androidx.compose.runtime.mutableStateOf
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import com.pcremote.MainActivity
import com.pcremote.R
import com.pcremote.home.*
import com.pcremote.nav.Mode
import com.pcremote.ui.theme.MidnightTheme
import org.junit.Assert.*
import org.junit.Test
import java.io.File

/** Synthetic UI only. No connection, real thumbnails or paired-device data. */
class DesktopLayoutInstrumentedTest {
    private fun matching(root:android.view.accessibility.AccessibilityNodeInfo?,label:String):List<android.view.accessibility.AccessibilityNodeInfo> {
        if(root==null)return emptyList()
        val queue=java.util.ArrayDeque<android.view.accessibility.AccessibilityNodeInfo>()
        val matches=mutableListOf<android.view.accessibility.AccessibilityNodeInfo>()
        queue.add(root)
        var visited=0
        // Compose exposes virtual children; its provider doesn't implement the
        // native findAccessibilityNodeInfosByText query. Traverse like UiAutomator.
        while(queue.isNotEmpty() && visited++<1000) {
            val node=queue.removeFirst()
            if(node.text?.toString()?.contains(label)==true)matches.add(node)
            for(i in 0 until node.childCount)node.getChild(i)?.let(queue::addLast)
        }
        return matches
    }
    @Test fun checkingAndConnectedKeepComputerActionsInView() {
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        val automation=instrumentation.uiAutomation
        val state=mutableStateOf(false)
        ActivityScenario.launch(MainActivity::class.java).use {scenario ->
            scenario.onActivity {activity ->
                activity.setContent {
                    MidnightTheme(dark=true,dynamic=false) {
                        HomeScreen(listOf(PcCardUi("layout-fixture","Review PC",state.value,false,Mode.entries,
                            if(state.value)PrimaryAction.OPEN else PrimaryAction.CHECKING,os="Windows")),emptyMap(),Mode.entries.toSet(),
                            HomeActions({_,_->},{},{},{},{},{_,_->},{}))
                    }
                }
            }
            for(online in listOf(false,true)) {
                scenario.onActivity{state.value=online}
                instrumentation.waitForIdleSync();Thread.sleep(350)
                val label=instrumentation.targetContext.getString(if(online)R.string.pcr_open_screen else R.string.pcr_checking)
                val until=System.currentTimeMillis()+3000
                var root=automation.rootInActiveWindow
                var actions=matching(root,label)
                while(actions.isEmpty() && System.currentTimeMillis()<until) {
                    Thread.sleep(50);root=automation.rootInActiveWindow;actions=matching(root,label)
                }
                val bitmap=automation.takeScreenshot()
                File(instrumentation.targetContext.filesDir,if(online)"home-connected.png" else "home-checking.png").outputStream().use {
                    assertTrue(bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it))
                }
                assertTrue("Computer action missing: $label",actions.isNotEmpty())
                val screen=Rect();root!!.getBoundsInScreen(screen)
                val minHeight=40*instrumentation.targetContext.resources.displayMetrics.density
                assertTrue("Computer action clipped below window",actions.any {node ->val bounds=Rect();node.getBoundsInScreen(bounds);screen.contains(bounds) && bounds.height()>=minHeight})
            }
        }
    }
}
