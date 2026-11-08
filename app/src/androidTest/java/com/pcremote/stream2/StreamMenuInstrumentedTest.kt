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
        val state=ToolbarState().apply {connected=true;canPip=true;canCaptureMouse=false}
        ActivityScenario.launch(MainActivity::class.java).use {scenario ->
            scenario.onActivity { activity ->activity.setContent {
                MidnightTheme(dark=true) {
                    Box(Modifier.fillMaxSize()) {
                        Box(Modifier.fillMaxSize().background(Color(0xff202a30)).clickable {remoteClicks.incrementAndGet()})
                        StreamOverlay(state,object:ToolbarActions {
                            override fun shortcut(s:Shortcut){}
                            override fun keyboard(){state.keyboardOpen=!state.keyboardOpen}
                            override fun setTrackpad(on:Boolean){}
                            override fun disconnect(){disconnected.incrementAndGet()}
                            override fun keys(events:List<KeyEvt>){}
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
        val state=ToolbarState().apply {connected=true;showPerf=true
            perfText=stripText(StripStats(1920,1080,60,"Direct",12,24500,59,"c2.qti.hevc.decoder"))}
        ActivityScenario.launch(MainActivity::class.java).use {scenario ->
            scenario.onActivity { activity ->activity.setContent {
                MidnightTheme(dark=true) {
                    Box(Modifier.fillMaxSize().background(Color(0xff202a30))) {
                        StreamOverlay(state,object:ToolbarActions {
                            override fun shortcut(s:Shortcut){}
                            override fun keyboard(){}
                            override fun setTrackpad(on:Boolean){}
                            override fun disconnect(){}
                            override fun keys(events:List<KeyEvt>){}
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
            assertNotNull("the saved 1280 × 720 is shown as current",find("1280 × 720"))
            mouseClick(find("1920 × 1080")!!)
            waitUntil {picked.get()==(1920 to 1080)}
            state.resizeNotice=true
            Thread.sleep(300)
            save("stream-resize-notice.png")
            state.resizeNotice=false;state.buttonHidden=true
            waitUntil {find("Stream menu")==null}
        }
    }
    @Test fun mainPageOrderGeometryShareClipboardAndAudioPage() {
        val density=InstrumentationRegistry.getInstrumentation().targetContext.resources.displayMetrics.density
        val shared=java.util.concurrent.atomic.AtomicReference<Boolean?>()
        val copied=java.util.concurrent.atomic.AtomicReference<String?>()
        val reopened=AtomicInteger();val picked=java.util.concurrent.atomic.AtomicReference<String?>()
        val state=ToolbarState().apply {connected=true;canPip=true;audioOn=true;showPerf=true
            displays=listOf(com.pcremote.agent.DisplayInfo("a","Display 1",true),com.pcremote.agent.DisplayInfo("b","Display 2"))
            display="a";perfText=stripText(StripStats(1280,720,60,"Direct",12,24500,59,"c2.qti.hevc.decoder"))}
        ActivityScenario.launch(MainActivity::class.java).use {scenario ->
            scenario.onActivity { activity ->activity.setContent {
                MidnightTheme(dark=true) {
                    Box(Modifier.fillMaxSize().background(Color(0xff202a30))) {
                        StreamOverlay(state,object:ToolbarActions {
                            override fun shortcut(s:Shortcut){}
                            override fun keyboard(){}
                            override fun setTrackpad(on:Boolean){}
                            override fun disconnect(){}
                            override fun keys(events:List<KeyEvt>){}
                            override fun selectDisplay(id:String){picked.set(id)}
                            override fun setShareClipboard(on:Boolean){shared.set(on)}
                            override fun audioDetails()="Output: Speaker\nsubmitted=12 decoded=12"
                            override fun copyText(text:String){copied.set(text)}
                            override fun reopenAudio(){reopened.incrementAndGet()}
                        })
                    }
                }
            } }
            waitUntil {find("Stream menu")!=null}
            val button=find("Stream menu")!!
            assertEquals("button is 40dp",40f*density,button.width().toFloat(),1f)
            assertEquals("button is 40dp",40f*density,button.height().toFloat(),1f)
            mouseClick(button)
            waitUntil {find("Disconnect")!=null}
            Thread.sleep(400)
            val labels=listOf("Keyboard","Trackpad","Picture in picture","Resolution","Switch display","Share clipboard","Stream stats","Audio","More keys","Help","Disconnect")
            val rects=labels.map {find(it) ?: error("missing row $it")}
            for(i in 1 until rects.size)assertTrue("${labels[i]} below ${labels[i-1]}",rects[i].top>=rects[i-1].bottom)
            for(i in rects.indices)assertEquals("row ${labels[i]} is 34dp",34f*density,rects[i].height().toFloat(),1.5f)
            for(i in 1 until 6)assertEquals("rows touch",rects[i-1].bottom.toFloat(),rects[i].top.toFloat(),1.5f)
            assertEquals("row width is 250dp",250f*density,rects[0].width().toFloat(),1.5f)
            assertEquals("panel is 54dp right of the button edge",54f*density,(rects[0].left-button.left).toFloat(),2f)
            save("stream-menu-main.png")
            mouseClick(rects[5])
            waitUntil {shared.get()==false}
            assertFalse(state.shareClipboard)
            Thread.sleep(300)
            save("stream-menu-share-off.png")
            mouseClick(find("Share clipboard")!!)
            waitUntil {shared.get()==true}
            mouseClick(find("Audio")!!)
            waitUntil {find("Copy details")!=null && find("Reopen output")!=null}
            Thread.sleep(1200)
            save("stream-menu-audio.png")
            clickUntil("Copy details") {copied.get()?.contains("Output: Speaker")==true}
            mouseClick(find("Reopen output")!!)
            waitUntil {reopened.get()==1}
            state.audioOn=false
            waitUntil {find("Copy details")==null}
            Thread.sleep(300)
            save("stream-menu-audio-off.png")
            state.audioOn=true
            mouseClick(find("Audio")!!)
            waitUntil {find("Audio")!=null && find("Keyboard")!=null}
            mouseClick(find("Switch display")!!)
            waitUntil {find("Display 2")!=null}
            save("stream-menu-displays.png")
            mouseClick(find("Display 2")!!)
            waitUntil {picked.get()=="b"}
            waitUntil {find("Share clipboard")!=null}
            mouseClick(find("More keys")!!)
            waitUntil {find("Media keys")!=null}
            save("stream-menu-more-keys.png")
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
    /** A mouse press that lands before the pointer has hovered the row is sometimes dropped: click again. */
    private fun clickUntil(description:String,done:()->Boolean) {
        repeat(3) {
            if(done())return
            find(description)?.let {mouseClick(it)}
            val end=SystemClock.uptimeMillis()+1000
            while(!done() && SystemClock.uptimeMillis()<end)Thread.sleep(50)
        }
        assertTrue(" click had no effect",done())
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
