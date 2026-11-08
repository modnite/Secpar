package com.pcremote.prs

import android.media.MediaCodec
import android.media.MediaFormat
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.util.Log
import android.view.Surface
import android.view.Choreographer
import java.nio.ByteBuffer
import java.util.ArrayDeque

/**
 * Hardware MediaCodec with bounded input and display-paced output. Configures itself on the first
 * keyframe (parameter sets become csd buffers). Obsolete decoded frames never queue on the surface.
 * [onError] fires once if the decoder can't handle the stream (caller falls back to another codec).
 */
class VideoDecoder(
    private val surface: Surface,
    private val codec: String,
    private val width: Int,
    private val height: Int,
    private val fps: Int,
    private val onError: (String) -> Unit,
    private val onNeedKeyframe: () -> Unit,
) {
    private val thread = HandlerThread("pcr-video", android.os.Process.THREAD_PRIORITY_URGENT_DISPLAY).apply { start() }
    private val handler = Handler(thread.looper)
    private val lock = Any()
    private val frames = VideoInputQueue(fps)
    private val pictures = VideoOutputQueue(fps)
    private val inputs = ArrayDeque<Int>()
    @Volatile private var mc: MediaCodec? = null
    private var configureScheduled = false
    private var choreographer: Choreographer? = null
    private var renderPosted = false
    private val feedPosted = java.util.concurrent.atomic.AtomicBoolean(false)
    @Volatile private var dead = false

    /** Average time from frame arrival to render, ms (for REPORT and stats). */
    @Volatile var decodeMs = 0; private set
    private var decodeAvgUs = 0.0
    private var outputDrops = 0
    private var presented = 0
    private var statsAtNs = System.nanoTime()

    /** The MediaCodec component in use, empty until the first keyframe configures it. */
    @Volatile var name = ""; private set

    /** Called on each rendered frame with the frame's arrival time (our clock, µs). */
    @Volatile var onRendered: ((receivedUs: Long) -> Unit)? = null

    fun submit(f: Frame) {
        if (dead) return
        synchronized(lock) {
            val admission = frames.offer(f, System.nanoTime() / 1000)
            if (admission.requestKey) onNeedKeyframe()
            if (!admission.accepted) return
            if (mc == null && !configureScheduled) {
                configureScheduled = true
                handler.post { configure(f) }
            }
        }
        if (feedPosted.compareAndSet(false, true)) handler.post { feedPosted.set(false); feed() }
    }

    fun release() {
        dead = true
        handler.post {
            choreographer?.removeFrameCallback(renderFrame)
            pictures.clear().forEach { picture -> runCatching { mc?.releaseOutputBuffer(picture.index, false) } }
            synchronized(lock) { frames.clear(); inputs.clear() }
            runCatching { mc?.stop() }; runCatching { mc?.release() }; mc = null
            thread.quitSafely()
        }
    }

    private fun configure(first: Frame) {
        if (mc != null || dead) return
        val mime = CodecPicker.MIME[codec] ?: return fail("unknown codec $codec")
        val info = CodecPicker.decoderFor(codec) ?: return fail("no decoder for $codec")
        val fmt = MediaFormat.createVideoFormat(mime, width, height).apply {
            if (Build.VERSION.SDK_INT >= 30) setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
            setInteger(MediaFormat.KEY_PRIORITY, 0)
            setInteger(MediaFormat.KEY_FRAME_RATE, fps)
            if (Build.VERSION.SDK_INT >= 29) setInteger(MediaFormat.KEY_ALLOW_FRAME_DROP, 1)
            setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, maxOf(width * height, 1 shl 20))
            if (Build.VERSION.SDK_INT >= 30) setFloat(MediaFormat.KEY_OPERATING_RATE, fps.toFloat())
            CodecPicker.vendorLowLatencyKeys(info.name).forEach { (k, v) -> setInteger(k, v) }
            csd(first.data, codec).forEachIndexed { i, b -> setByteBuffer("csd-$i", ByteBuffer.wrap(b)) }
        }
        try {
            val c = MediaCodec.createByCodecName(info.name)
            c.setCallback(callback, handler)
            try { c.configure(fmt, surface, null, 0) } catch (e: Exception) {
                // some decoders reject the vendor keys; retry plain before giving up
                Log.w(TAG, "configure with low-latency keys failed, retrying plain", e)
                c.reset(); c.setCallback(callback, handler)
                CodecPicker.vendorLowLatencyKeys(info.name).keys.forEach { fmt.removeKey(it) }
                if (Build.VERSION.SDK_INT >= 30) { fmt.removeKey(MediaFormat.KEY_LOW_LATENCY); fmt.removeKey(MediaFormat.KEY_OPERATING_RATE) }
                c.configure(fmt, surface, null, 0)
            }
            c.start()
            mc = c
            name = info.name
            choreographer = Choreographer.getInstance()
            if (Build.VERSION.SDK_INT >= 30) runCatching {
                surface.setFrameRate(fps.toFloat(), Surface.FRAME_RATE_COMPATIBILITY_FIXED_SOURCE)
            }
            Log.i(TAG, "decoder ${info.name} $codec ${width}x$height@$fps lowLatency=${info.lowLatency}")
        } catch (e: Exception) {
            fail("decoder ${info.name}: ${e.message}")
        }
    }

    private fun feed() {
        val c = mc ?: return
        while (true) {
            val (idx, f) = synchronized(lock) {
                if (dead || inputs.isEmpty()) return
                val nowUs = System.nanoTime() / 1000
                val frame = frames.poll(nowUs)
                if (frames.needsKey(nowUs)) onNeedKeyframe()
                if (frame == null) return
                inputs.removeFirst() to frame
            }
            try {
                val buf = c.getInputBuffer(idx) ?: return fail("decoder input buffer unavailable")
                buf.clear()
                if (f.data.size > buf.remaining()) {
                    c.queueInputBuffer(idx, 0, 0, 0, 0)
                    synchronized(lock) { if (frames.invalidate(System.nanoTime() / 1000)) onNeedKeyframe() }
                    continue
                }
                buf.put(f.data)
                c.queueInputBuffer(idx, 0, f.data.size, f.receivedUs, if (f.header.keyframe) MediaCodec.BUFFER_FLAG_KEY_FRAME else 0)
            } catch (e: IllegalStateException) { return fail("decoder input: ${e.message}") }
        }
    }

    private val callback = object : MediaCodec.Callback() {
        override fun onInputBufferAvailable(codec: MediaCodec, index: Int) {
            if (dead) return
            synchronized(lock) { inputs.addLast(index) }
            feed()
        }

        override fun onOutputBufferAvailable(codec: MediaCodec, index: Int, info: MediaCodec.BufferInfo) {
            if (dead) { runCatching { codec.releaseOutputBuffer(index, false) }; return }
            pictures.offer(VideoOutputQueue.Picture(index, info.presentationTimeUs))?.let {
                outputDrops++
                runCatching { codec.releaseOutputBuffer(it.index, false) }
            }
            if (!renderPosted) {
                renderPosted = true
                choreographer?.postFrameCallback(renderFrame)
            }
        }

        override fun onError(codec: MediaCodec, e: MediaCodec.CodecException) {
            if (e.isRecoverable) { onNeedKeyframe(); return }
            fail("decoder error: ${e.diagnosticInfo}")
        }

        override fun onOutputFormatChanged(codec: MediaCodec, format: MediaFormat) {}
    }

    private val renderFrame = object : Choreographer.FrameCallback {
        override fun doFrame(frameTimeNanos: Long) {
            renderPosted = false
            if (dead) return
            pictures.poll(frameTimeNanos)?.let { picture ->
                try {
                    mc?.releaseOutputBuffer(picture.index, frameTimeNanos)
                    presented++
                    if (picture.receivedUs > 0) {
                        val took = (System.nanoTime() / 1000 - picture.receivedUs).coerceIn(0, 1_000_000)
                        decodeAvgUs += (took - decodeAvgUs) / 16
                        decodeMs = (decodeAvgUs / 1000).toInt()
                        onRendered?.invoke(picture.receivedUs)
                    }
                    val now = System.nanoTime()
                    if (now - statsAtNs >= 5_000_000_000L) {
                        val shownFps = presented * 1_000_000_000.0 / (now - statsAtNs)
                        val resyncs = synchronized(lock) { frames.resyncs }
                        Log.i(TAG, "pacing shownFps=${shownFps.toInt()} queueDrops=$outputDrops inputResyncs=$resyncs decodeMs=$decodeMs")
                        statsAtNs = now
                        presented = 0
                        outputDrops = 0
                    }
                } catch (error: IllegalStateException) { fail("decoder render: ${error.message}") }
            }
            if (!dead && !pictures.isEmpty()) {
                renderPosted = true
                choreographer?.postFrameCallback(this)
            }
        }
    }

    private fun fail(why: String) {
        if (dead) return
        dead = true
        Log.e(TAG, why)
        onError(why)
    }

    companion object {
        private const val TAG = "PcrVideo"

        /** Annex B NAL units (start codes stripped). */
        fun nals(b: ByteArray): List<IntRange> {
            val starts = ArrayList<Pair<Int, Int>>() // (start-code index, payload index)
            var i = 0
            while (i + 3 <= b.size) {
                if (b[i].toInt() == 0 && b[i + 1].toInt() == 0) {
                    if (b[i + 2].toInt() == 1) { starts += i to i + 3; i += 3; continue }
                    if (i + 4 <= b.size && b[i + 2].toInt() == 0 && b[i + 3].toInt() == 1) { starts += i to i + 4; i += 4; continue }
                }
                i++
            }
            return starts.mapIndexed { n, (_, p) -> p until (starts.getOrNull(n + 1)?.first ?: b.size) }
        }

        /** Codec-specific data from a keyframe: H.264 → [SPS, PPS]; HEVC → [VPS+SPS+PPS]; AV1 → none. */
        fun csd(frame: ByteArray, codec: String = ""): List<ByteArray> {
            val sc = byteArrayOf(0, 0, 0, 1)
            val units = nals(frame).filter { !it.isEmpty() }
            val h264 = units.filter { (frame[it.first].toInt() and 0x1F) in 7..8 }
            val hevc = units.filter { ((frame[it.first].toInt() shr 1) and 0x3F) in 32..34 }
            fun join(rs: List<IntRange>) = rs.fold(ByteArray(0)) { acc, r -> acc + sc + frame.copyOfRange(r.first, r.last + 1) }
            return when {
                codec == "av1" -> emptyList()
                hevc.size >= 3 && codec != "h264" -> listOf(join(hevc))
                h264.size >= 2 -> listOf(join(h264.filter { (frame[it.first].toInt() and 0x1F) == 7 }), join(h264.filter { (frame[it.first].toInt() and 0x1F) == 8 }))
                else -> emptyList()
            }
        }
    }
}
