package com.pcremote.scan

import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Color
import android.os.Handler
import android.os.Looper
import android.view.PixelCopy
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.pcremote.prs.Frame
import com.pcremote.prs.FrameHeader
import com.pcremote.prs.VideoDecoder
import com.pcremote.review.VideoReviewActivity
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.ByteArrayOutputStream
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

@RunWith(AndroidJUnit4::class)
class VideoPacingInstrumentedTest {
    @Test fun realDecoderPresents120FpsInputOnAVsyncPacedSurface() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val activity = instrumentation.startActivitySync(Intent(instrumentation.targetContext, VideoReviewActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as VideoReviewActivity
        assertTrue(activity.surfaceReady.await(10, TimeUnit.SECONDS))
        val encoded = instrumentation.context.assets.open("video-pacing.h264").use { it.readBytes() }
        val pictures = mutableListOf<ByteArray>()
        var accessUnit = ByteArrayOutputStream()
        for (nal in VideoDecoder.nals(encoded)) {
            if (nal.isEmpty()) continue
            if (encoded[nal.first].toInt() and 31 == 9 && accessUnit.size() > 0) {
                pictures.add(accessUnit.toByteArray()); accessUnit = ByteArrayOutputStream()
            }
            accessUnit.write(byteArrayOf(0, 0, 0, 1))
            accessUnit.write(encoded, nal.first, nal.count())
        }
        if (accessUnit.size() > 0) pictures.add(accessUnit.toByteArray())
        assertTrue(pictures.size >= 120)
        val errors = CopyOnWriteArrayList<String>()
        val displayed = AtomicInteger()
        val decoder = VideoDecoder(activity.video.holder.surface, "h264", 320, 180, 120,
            { errors.add(it) }, {})
        decoder.onRendered = { displayed.incrementAndGet() }
        try {
            for ((id, data) in pictures.take(180).withIndex()) {
                val now = System.nanoTime() / 1000
                val key = VideoDecoder.nals(data).any { !it.isEmpty() && data[it.first].toInt() and 31 == 5 }
                decoder.submit(Frame(id, FrameHeader(data.size, now, key, 1), data, now))
                Thread.sleep(8)
            }
            assertTrue("Decoder errors: $errors", errors.isEmpty())
            assertTrue("No display-paced output", displayed.get() >= 5)
            val bitmap = Bitmap.createBitmap(320, 180, Bitmap.Config.ARGB_8888)
            val copied = CountDownLatch(1)
            val copyResult = AtomicInteger(-1)
            PixelCopy.request(activity.video, bitmap, { copyResult.set(it); copied.countDown() }, Handler(Looper.getMainLooper()))
            assertTrue(copied.await(5, TimeUnit.SECONDS))
            assertEquals(PixelCopy.SUCCESS, copyResult.get())
            val pixel = bitmap.getPixel(160, 90)
            assertTrue("Decoded blue test picture was not presented", Color.blue(pixel) > 120 && Color.blue(pixel) > Color.red(pixel) + 60)
            bitmap.recycle()
        } finally {
            decoder.release()
            Thread.sleep(150)
            instrumentation.runOnMainSync { activity.finish() }
        }
    }
}
