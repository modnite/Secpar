package com.pcremote.review

import android.os.Bundle
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.activity.ComponentActivity
import java.util.concurrent.CountDownLatch

/** Debug-only real output surface, without accounts, host connections or audio. */
class VideoReviewActivity : ComponentActivity() {
    lateinit var video: SurfaceView
    val surfaceReady = CountDownLatch(1)
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        video = SurfaceView(this)
        video.holder.setFixedSize(320, 180)
        video.holder.addCallback(object : SurfaceHolder.Callback {
            override fun surfaceCreated(holder: SurfaceHolder) { surfaceReady.countDown() }
            override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {}
            override fun surfaceDestroyed(holder: SurfaceHolder) {}
        })
        setContentView(video)
    }
}
