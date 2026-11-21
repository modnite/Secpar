package com.pcremote.prs

import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Color
import android.os.Handler
import android.os.Looper
import android.view.PixelCopy
import androidx.test.platform.app.InstrumentationRegistry
import com.pcremote.review.VideoReviewActivity
import java.util.concurrent.CountDownLatch
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.util.concurrent.TimeUnit
import kotlin.math.roundToInt

/**
 * Full colour end to end on this device's codecs and OpenGL: a packed frame of one-pixel red and
 * blue columns is encoded, decoded into [PackedRenderer], and the drawn pixels are read back.
 * 4:2:0 colour would average each red and blue pair into purple.
 */
@RunWith(AndroidJUnit4::class)
class PackedRendererInstrumentedTest {
    private val w = 64
    private val h = 36

    private fun ycc(r: Double, g: Double, b: Double): IntArray {
        val l = 0.2126 * r + 0.7152 * g + 0.0722 * b
        return intArrayOf((16 + 219 * l).roundToInt(), (128 + 224 * (b - l) / 1.8556).roundToInt(), (128 + 224 * (r - l) / 1.5748).roundToInt())
    }

    /** The packed frame (w by 2h, 4:2:0 planes) for red even columns and blue odd ones. */
    private fun packed(): Triple<ByteArray, ByteArray, ByteArray> {
        val y = ByteArray(w * h * 2)
        val u = ByteArray(w / 2 * h)
        val v = ByteArray(w / 2 * h)
        for (row in 0 until h) for (x in 0 until w) {
            val (l, cb, cr) = if (x % 2 == 0) ycc(1.0, 0.0, 0.0) else ycc(0.0, 0.0, 1.0)
            y[row * w + x] = l.toByte()
            y[(row + h) * w + x] = cb.toByte()
            (if (x % 2 == 0) u else v)[row * (w / 2) + x / 2] = cr.toByte()
        }
        return Triple(y, u, v)
    }

    /** Encodes the frame until the encoder gives back its configuration and a keyframe. */
    private fun encode(): Pair<List<ByteArray>, ByteArray> {
        val (y, u, v) = packed()
        val format = MediaFormat.createVideoFormat("video/avc", w, h * 2).apply {
            setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible)
            setInteger(MediaFormat.KEY_BIT_RATE, 8_000_000)
            setInteger(MediaFormat.KEY_FRAME_RATE, 30)
            setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1)
        }
        val enc = MediaCodec.createEncoderByType("video/avc")
        enc.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        enc.start()
        val config = mutableListOf<ByteArray>()
        val info = MediaCodec.BufferInfo()
        var pts = 0L
        repeat(200) {
            val inIdx = enc.dequeueInputBuffer(10_000)
            if (inIdx >= 0) {
                val image = enc.getInputImage(inIdx)!!
                val planes = image.planes
                for (row in 0 until h * 2) for (x in 0 until w) planes[0].buffer.put(row * planes[0].rowStride + x * planes[0].pixelStride, y[row * w + x])
                for (row in 0 until h) for (x in 0 until w / 2) {
                    planes[1].buffer.put(row * planes[1].rowStride + x * planes[1].pixelStride, u[row * (w / 2) + x])
                    planes[2].buffer.put(row * planes[2].rowStride + x * planes[2].pixelStride, v[row * (w / 2) + x])
                }
                enc.queueInputBuffer(inIdx, 0, w * h * 3, pts, 0)
                pts += 33_333
            }
            val outIdx = enc.dequeueOutputBuffer(info, 10_000)
            if (outIdx >= 0) {
                val out = enc.getOutputBuffer(outIdx)!!
                val bytes = ByteArray(info.size).also { out.get(it) }
                enc.releaseOutputBuffer(outIdx, false)
                if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) config += bytes
                else if (info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0) { enc.stop(); enc.release(); return config to bytes }
            }
        }
        error("the encoder gave no keyframe")
    }

    /** Draws onto the review activity's 320 by 180 video view, as the app draws onto its stream view. */
    @Test fun fullColourKeepsNeighbouringPixelsApart() {
        val (config, key) = encode()
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val activity = instrumentation.startActivitySync(Intent(instrumentation.targetContext, VideoReviewActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as VideoReviewActivity
        assertTrue(activity.surfaceReady.await(10, TimeUnit.SECONDS))
        var error: String? = null
        val renderer = PackedRenderer(activity.video.holder.surface, w, h) { error = it }
        // As the app does: no surface, frames taken from the decoder's own buffers.
        val dec = MediaCodec.createDecoderByType("video/avc")
        dec.configure(MediaFormat.createVideoFormat("video/avc", w, h * 2).apply {
            setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible)
        }, null, null, 0)
        dec.start()
        val info = MediaCodec.BufferInfo()
        // The end of the stream makes a software decoder hand over what it holds.
        val inputs = config.map { it to MediaCodec.BUFFER_FLAG_CODEC_CONFIG } + List(4) { key to MediaCodec.BUFFER_FLAG_KEY_FRAME } +
            listOf(ByteArray(0) to MediaCodec.BUFFER_FLAG_END_OF_STREAM)
        var next = 0
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(10)
        while (renderer.drawn == 0 && System.nanoTime() < deadline && error == null) {
            if (next < inputs.size) {
                val i = dec.dequeueInputBuffer(10_000)
                if (i >= 0) { val (b, flags) = inputs[next++]; dec.getInputBuffer(i)!!.put(b); dec.queueInputBuffer(i, 0, b.size, next * 33_333L, flags) }
            }
            val o = dec.dequeueOutputBuffer(info, 10_000)
            if (o >= 0) { dec.getOutputImage(o)?.let(renderer::frame); dec.releaseOutputBuffer(o, false) }
        }
        Thread.sleep(300)
        val bitmap = Bitmap.createBitmap(320, 180, Bitmap.Config.ARGB_8888)
        val copied = CountDownLatch(1)
        var result = -1
        PixelCopy.request(activity.video, bitmap, { result = it; copied.countDown() }, Handler(Looper.getMainLooper()))
        assertTrue(copied.await(5, TimeUnit.SECONDS))
        dec.stop(); dec.release()
        renderer.release()
        activity.finish()
        assertTrue("a drawn frame (renderer: $error, drawn ${renderer.drawn}, copy $result)", renderer.drawn > 0 && result == PixelCopy.SUCCESS)
        // 64 picture pixels over 320 view pixels: view column 5k+2 is the centre of picture column k.
        val red = bitmap.getPixel(5 * 6 + 2, 90)
        val blue = bitmap.getPixel(5 * 7 + 2, 90)
        assertTrue("red stays red: ${Integer.toHexString(red)}", Color.red(red) > 200 && Color.blue(red) < 60)
        assertTrue("blue stays blue: ${Integer.toHexString(blue)}", Color.blue(blue) > 200 && Color.red(blue) < 60)
    }
}
