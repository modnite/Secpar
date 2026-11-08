package com.pcremote.stream2

import android.graphics.Rect
import android.os.SystemClock
import android.view.InputDevice
import android.view.MotionEvent
import android.view.accessibility.AccessibilityNodeInfo
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import com.pcremote.MainActivity
import com.pcremote.stream.*
import com.pcremote.ui.theme.MidnightTheme
import org.junit.Assert.*
import org.junit.Test
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

class StreamMenuInstrumentedTest {
    @Test fun dockMouseOpensCornerMenuWithoutClickingRemotePicture() {
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        val remoteClicks=AtomicInteger();val disconnected=AtomicInteger();val localUi=AtomicBoolean()
        val state=ToolbarState().apply {connected=true;canPip=true;canCaptureMouse=false;coachShown=true}
        ActivityScenario.launch(MainActivity::class.java).use {scenario ->
            scenario.onActivity { activity ->activity.setContent {
                MidnightTheme(dark=true,dynamic=false) {
                    Box(Modifier.fillMaxSize()) {
                        Box(Modifier.fillMaxSize().background(Color(0xff202a30)).clickable {remoteClicks.incrementAndGet()})
                        StreamOverlay(state,object:ToolbarActions {
                            override fun shortcut(s:Shortcut){}
                            override fun keyboard(){state.keyboardOpen=!state.keyboardOpen}
                            override fun setTrackpad(on:Boolean){}
                            override fun lowerQuality(){}
                            override fun disconnect(){disconnected.incrementAndGet()}
                            override fun keys(events:List<KeyEvt>){}
                            override fun coachDone(){}
                            override fun selectDisplay(id:String){}
                        }) {localUi.set(it)}
                    }
                }
            } }
            waitUntil {find("Stream menu")!=null}
            save("stream-corner-button.png")
            mouseClick(find("Stream menu")!!)
            waitUntil {localUi.get() && find("Disconnect")!=null}
            assertEquals(0,remoteClicks.get())
            save("stream-corner-menu.png")
            mouseClick(find("Stream menu")!!)
            waitUntil {!localUi.get() && find("Disconnect")==null}
            mouseClick(find("Stream menu")!!)
            waitUntil {find("Disconnect")!=null}
            mouseClick(find("Disconnect")!!)
            waitUntil {disconnected.get()==1}
            assertEquals("Menu mouse clicks must remain local",0,remoteClicks.get())
        }
    }
    @Test fun statsStripStaysWhileResolutionPageIsOpenAndPicksASize() {
        val picked=java.util.concurrent.atomic.AtomicReference<Pair<Int,Int>?>()
        val state=ToolbarState().apply {connected=true;coachShown=true;showPerf=true
            perfText=stripText(StripStats(1920,1080,60,"Direct",12,24500,59,"c2.qti.hevc.decoder"))}
        ActivityScenario.launch(MainActivity::class.java).use {scenario ->
            scenario.onActivity { activity ->activity.setContent {
                MidnightTheme(dark=true,dynamic=false) {
                    Box(Modifier.fillMaxSize().background(Color(0xff202a30))) {
                        StreamOverlay(state,object:ToolbarActions {
                            override fun shortcut(s:Shortcut){}
                            override fun keyboard(){}
                            override fun setTrackpad(on:Boolean){}
                            override fun lowerQuality(){}
                            override fun disconnect(){}
                            override fun keys(events:List<KeyEvt>){}
                            override fun coachDone(){}
                            override fun selectDisplay(id:String){}
                            override fun selectSize(w:Int,h:Int){picked.set(w to h)}
                        })
                    }
                }
            } }
            waitUntil {find("Stream menu")!=null}
            save("stream-strip.png")
            mouseClick(find("Stream menu")!!)
            waitUntil {find("Resolution")!=null}
            mouseClick(find("Resolution")!!)
            waitUntil {find("1280 × 720")!=null}
            save("stream-resolution-page.png")
            mouseClick(find("1280 × 720")!!)
            waitUntil {picked.get()==(1280 to 720)}
            state.resizeNotice=true
            Thread.sleep(300)
            save("stream-resize-notice.png")
            state.resizeNotice=false;state.buttonHidden=true
            waitUntil {find("Stream menu")==null}
        }
    }
    private fun find(description:String):Rect? {
        fun scan(node:AccessibilityNodeInfo):Rect? {
            if(node.contentDescription?.toString()==description)return Rect().also {node.getBoundsInScreen(it)}
            for(i in 0 until node.childCount)node.getChild(i)?.let {scan(it)?.let {r->return r}}
            return null
        }
        return InstrumentationRegistry.getInstrumentation().uiAutomation.rootInActiveWindow?.let(::scan)
    }
    private fun mouseClick(rect:Rect) {
        val automation=InstrumentationRegistry.getInstrumentation().uiAutomation
        val time=SystemClock.uptimeMillis()
        for(action in listOf(MotionEvent.ACTION_DOWN,MotionEvent.ACTION_UP)) {
            val event=MotionEvent.obtain(time,SystemClock.uptimeMillis(),action,1,
                arrayOf(MotionEvent.PointerProperties().apply {id=0;toolType=MotionEvent.TOOL_TYPE_MOUSE}),
                arrayOf(MotionEvent.PointerCoords().apply {x=rect.exactCenterX();y=rect.exactCenterY();pressure=1f;size=1f}),
                0,if(action==MotionEvent.ACTION_DOWN)MotionEvent.BUTTON_PRIMARY else 0,1f,1f,0,0,InputDevice.SOURCE_MOUSE,0)
            assertTrue(automation.injectInputEvent(event,true));event.recycle()
        }
    }
    private fun save(name:String) {
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        instrumentation.waitForIdleSync()
        val image=instrumentation.uiAutomation.takeScreenshot()
        File(instrumentation.targetContext.filesDir,name).outputStream().use {assertTrue(image.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it))}
    }
    private fun waitUntil(test:()->Boolean) {
        val end=SystemClock.uptimeMillis()+6000
        while(!test() && SystemClock.uptimeMillis()<end)Thread.sleep(50)
        assertTrue("Stream menu did not settle",test())
    }
}
