package com.pcremote.prs

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.media.MediaCodec
import android.media.MediaFormat
import android.util.Log
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.Executors

/** Opus (48 kHz stereo, 10 ms packets) → MediaCodec → low-latency AudioTrack, with a 30–60 ms buffer. */
class AudioPlayer(private val captureToLocalUs: (Long) -> Long? = { it }) {
    private val exec = Executors.newSingleThreadScheduledExecutor { task -> Thread({
        android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_AUDIO)
        task.run()
    }, "pcr-audio") }
    private var codec: MediaCodec? = null
    private var pump: java.util.concurrent.ScheduledFuture<*>? = null
    @Volatile private var track: AudioTrack? = null
    private var writer = PcmWriter()
    private var head = PlaybackHead()
    private var playing = false
    private var lastUnderruns = 0
    private var fading = true
    private var previousLeft = 0
    private var previousRight = 0
    private var gapCaptureUs: Long? = null
    private var firstBufferedCaptureUs: Long? = null
    private var lastPacketId: Int? = null
    @Volatile internal var recoveries = 0; private set
    @Volatile internal var staleFrames = 0; private set
    @Volatile internal var lastQueuedAgeUs = 0L; private set
    private var statsAt = System.nanoTime()
    private var inputWaits = 0
    private var inputAgeUs = 0L
    private var decodedAgeUs = 0L
    private val timeline = PcmTimeline()
    private var codecTimestampOffsetUs = 0L
    @Volatile internal var timelineMisses = 0; private set
    private val pending = java.util.concurrent.atomic.AtomicInteger()
    private val overflowPackets = java.util.concurrent.atomic.AtomicInteger()
    @Volatile private var firstOverflow = "none"
    @Volatile internal var acceptedFrames = 0L; private set
    @Volatile internal var decodedFrames = 0L; private set
    @Volatile internal var submittedPackets = 0L; private set
    @Volatile internal var incompleteWrites = 0; private set
    @Volatile internal var audioFailures = 0; private set
    private var failureStreak = 0
    private var retryAfterNs = 0L
    private var healthySinceNs = 0L
    private var recoveryDrops = 0
    internal fun diagnostics() = "submitted=$submittedPackets decoded=$decodedFrames accepted=$acceptedFrames overflow=${overflowPackets.get()} firstOverflow=[$firstOverflow] stale=$staleFrames incomplete=$incompleteWrites timelineMisses=$timelineMisses recoveries=$recoveries audioFailures=$audioFailures recoveryDrops=$recoveryDrops pending=${pending.get()} codecTimestampOffsetUs=$codecTimestampOffsetUs"
    private val info = MediaCodec.BufferInfo()
    private var pcm = ByteArray(0)
    @Volatile private var dead = false

    /** Prepare before announcing a stream peer, so codec startup cannot backlog live packets. */
    fun prepare() {
        check(android.os.Looper.myLooper() != android.os.Looper.getMainLooper()) {
            "Audio preparation must run off the UI thread"
        }
        check(!dead) { "Audio player released" }
        exec.submit {
            check(!dead) { "Audio player released" }
            runCatching { if (codec == null) start() }
                .onFailure { recoverAudio(it) }.getOrThrow()
        }.get(2, java.util.concurrent.TimeUnit.SECONDS)
    }

    fun play(a: AudioPacket) {
        if (dead) return
        // Bound encoded backlog as well as the hardware queue after a network stall.
        if (pending.incrementAndGet() > 12) {
            pending.decrementAndGet()
            if (overflowPackets.incrementAndGet() == 1) {
                firstOverflow = "id=${a.id} submitted=$submittedPackets decoded=$decodedFrames"
                Log.w(TAG, "encoded backlog overflow: $firstOverflow")
            }
            return
        }
        runCatching { exec.execute {
            try { runCatching { decode(a) }.onFailure { recoverAudio(it) } }
            finally { pending.decrementAndGet() }
        } }.onFailure { pending.decrementAndGet() }
    }

    fun release() {
        dead = true
        // Pause wakes a blocking write, so release cannot wait behind output backpressure.
        runCatching { track?.pause() }
        runCatching { exec.execute { shutdown() } }
        exec.shutdown()
    }

    private fun start() {
        val fmt = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_OPUS, RATE, 2).apply {
            setInteger(MediaFormat.KEY_PCM_ENCODING, AudioFormat.ENCODING_PCM_16BIT)
            setByteBuffer("csd-0", ByteBuffer.wrap(opusHead()))
            setByteBuffer("csd-1", ByteBuffer.allocate(8).order(ByteOrder.nativeOrder()).putLong(0).flip() as ByteBuffer)
            setByteBuffer("csd-2", ByteBuffer.allocate(8).order(ByteOrder.nativeOrder()).putLong(0).flip() as ByteBuffer)
        }
        // Keep partially initialized resources reachable by audio-only cleanup too.
        val decoder = MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_AUDIO_OPUS)
        codec = decoder
        decoder.configure(fmt, null, null, 0)
        decoder.start()
        val min = AudioTrack.getMinBufferSize(RATE, AudioFormat.CHANNEL_OUT_STEREO, AudioFormat.ENCODING_PCM_16BIT)
        track = AudioTrack.Builder()
            .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_GAME).setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build())
            .setAudioFormat(AudioFormat.Builder().setSampleRate(RATE).setChannelMask(AudioFormat.CHANNEL_OUT_STEREO).setEncoding(AudioFormat.ENCODING_PCM_16BIT).build())
            .setBufferSizeInBytes(bufferBytes(min))
            .setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY)
            .setTransferMode(AudioTrack.MODE_STREAM)
            .build()
        if (android.os.Build.VERSION.SDK_INT >= 31) track!!.setStartThresholdInFrames(PRIME_FRAMES)
        Log.i(TAG, "start decoder=${codec!!.name} bufferFrames=${track!!.bufferSizeInFrames} primeFrames=$PRIME_FRAMES")
        pump = exec.scheduleWithFixedDelay({
            if (!dead) runCatching { checkUnderrun(track!!); drain(codec!!, track!!, 0) }
                .onFailure { recoverAudio(it) }
        }, 5, 5, java.util.concurrent.TimeUnit.MILLISECONDS)
    }

    private fun decode(packet: AudioPacket) {
        val opus = packet.opus
        if (dead) return
        if (System.nanoTime() < retryAfterNs) { recoveryDrops++; return }
        inputAgeUs = captureToLocalUs(packet.captureUs)?.let { System.nanoTime() / 1000 - it } ?: 0
        if (lastPacketId?.let { packet.id - it <= 0 } == true) return
        if (lastPacketId?.let { packet.id - it > 1 } == true) gapCaptureUs = packet.captureUs
        lastPacketId = packet.id
        if (codec == null) start()
        val c = codec!!
        val t = track!!
        // Free decoded output before waiting for another input slot. Never silently
        // discard an Opus packet because the asynchronous decoder is briefly busy.
        checkUnderrun(t)
        drain(c, t, 0)
        val deadline = System.nanoTime() + 100_000_000
        var inIdx = c.dequeueInputBuffer(5_000)
        while (inIdx < 0 && !dead) {
            inputWaits++
            drain(c, t, 1_000)
            check(System.nanoTime() < deadline) { "Audio decoder stopped accepting packets" }
            inIdx = c.dequeueInputBuffer(5_000)
        }
        if (dead) return
        c.getInputBuffer(inIdx)!!.apply { clear(); put(opus) }
        c.queueInputBuffer(inIdx, 0, opus.size, packet.captureUs, 0)
        timeline.queued(packet.captureUs)
        submittedPackets++
        drain(c, t, 1_000)
        if (System.nanoTime() - statsAt >= 5_000_000_000) {
            val queued = queuedFrames(t)
            Log.i(TAG, "queuedMs=${queued * 1000 / RATE} underruns=${t.underrunCount} shortWrites=${writer.shortWrites} zeroWrites=${writer.zeroWrites} inputWaits=$inputWaits pendingPackets=${pending.get()} overflowPackets=${overflowPackets.get()} recoveries=$recoveries audioFailures=$audioFailures recoveryDrops=$recoveryDrops staleFrames=$staleFrames captureAgeMs=${lastQueuedAgeUs / 1000} inputAgeMs=${inputAgeUs / 1000} decodedAgeMs=${decodedAgeUs / 1000} codecTimestampOffsetMs=${codecTimestampOffsetUs / 1000} timelineMisses=$timelineMisses")
            statsAt = System.nanoTime()
        }
    }

    private fun drain(c: MediaCodec, t: AudioTrack, timeoutUs: Long) {
        while (!dead) {
            val outIdx = c.dequeueOutputBuffer(info, timeoutUs)
            if (outIdx == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) continue
            if (outIdx < 0) break
            try {
                if (info.size == 0 || info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) continue
                val buf = c.getOutputBuffer(outIdx)!!
                if (pcm.size < info.size) pcm = ByteArray(info.size)
                buf.position(info.offset); buf.get(pcm, 0, info.size)
                val timedCaptureUs = timeline.consume(info.size / 4)
                decodedFrames += info.size / 4
                if (timedCaptureUs == null) timelineMisses++
                val hostCaptureUs = timedCaptureUs ?: info.presentationTimeUs
                codecTimestampOffsetUs = info.presentationTimeUs - hostCaptureUs
                val captureUs = captureToLocalUs(hostCaptureUs)
                val nowUs = System.nanoTime() / 1000
                decodedAgeUs = captureUs?.let { nowUs - it } ?: 0
                if (!playing && firstBufferedCaptureUs?.let { nowUs - it > AudioPlayout.MAX_LATENCY_US } == true) resetOutput(t)
                val queued = queuedFrames(t)
                val decision = AudioPlayout.decide(captureUs, nowUs, queued, false)
                if (decision.reset && (playing || writer.writtenFrames > 0)) resetOutput(t)
                if (decision.discard) { staleFrames++; continue }
                if (gapCaptureUs?.let { hostCaptureUs >= it } == true) {
                    PcmFade.bridge(pcm, info.size, previousLeft, previousRight)
                    gapCaptureUs = null
                } else if (fading) PcmFade.bridge(pcm, info.size)
                fading = false
                val before = writer.writtenFrames
                val deadlineUs = captureUs?.plus(AudioPlayout.MAX_LATENCY_US) ?: (nowUs + AudioPlayout.MAX_LATENCY_US)
                writer.write(pcm, info.size, { !dead && System.nanoTime() / 1000 <= deadlineUs }, { Thread.sleep(1) }) { data, offset, size ->
                    t.write(data, offset, size, AudioTrack.WRITE_NON_BLOCKING)
                }
                val accepted = writer.writtenFrames - before
                acceptedFrames += accepted
                if (accepted > 0) {
                    val nowNs = System.nanoTime()
                    if (healthySinceNs == 0L) healthySinceNs = nowNs
                    if (nowNs - healthySinceNs >= 1_000_000_000L) failureStreak = 0
                }
                if (!dead && accepted * 4 < info.size) { incompleteWrites++; staleFrames++; resetOutput(t); continue }
                if (info.size >= 4) {
                    previousLeft = PcmFade.sample(pcm, info.size - 4)
                    previousRight = PcmFade.sample(pcm, info.size - 2)
                }
                if (firstBufferedCaptureUs == null) firstBufferedCaptureUs = captureUs
                lastQueuedAgeUs = captureUs?.let { (System.nanoTime() / 1000 - it).coerceAtLeast(0) } ?: 0
                if (!playing && !dead && writer.writtenFrames >= PRIME_FRAMES) {
                    t.play()
                    playing = true
                }
            } finally { c.releaseOutputBuffer(outIdx, false) }
        }
    }

    private fun queuedFrames(t: AudioTrack) = if (playing) (writer.writtenFrames - head.frames(t.playbackHeadPosition)).coerceAtLeast(0) else writer.writtenFrames

    private fun checkUnderrun(t: AudioTrack) {
        val count = t.underrunCount
        if (playing && count > lastUnderruns) resetOutput(t)
        lastUnderruns = count
    }

    private fun resetOutput(t: AudioTrack) {
        t.pause()
        t.flush()
        writer = PcmWriter()
        head = PlaybackHead()
        playing = false
        fading = true
        previousLeft = 0; previousRight = 0
        firstBufferedCaptureUs = null
        lastUnderruns = t.underrunCount
        recoveries++
    }

    private fun closeAudio() {
        pump?.cancel(false); pump = null
        runCatching { track?.stop() }; runCatching { track?.release() }; track = null
        runCatching { codec?.stop() }; runCatching { codec?.release() }; codec = null
        timeline.clear()
        writer = PcmWriter()
        head = PlaybackHead()
        playing = false
        fading = true
        previousLeft = 0; previousRight = 0
        firstBufferedCaptureUs = null
        gapCaptureUs = null
        lastUnderruns = 0
    }

    private fun recoverAudio(failure: Throwable) {
        closeAudio()
        if (dead) return
        audioFailures++
        failureStreak = (failureStreak + 1).coerceAtMost(6)
        val retryMs = (50L shl (failureStreak - 1)).coerceAtMost(1000)
        retryAfterNs = System.nanoTime() + retryMs * 1_000_000
        healthySinceNs = 0
        Log.w(TAG, "audio restart retryMs=$retryMs failures=$audioFailures", failure)
    }

    private fun shutdown() {
        dead = true
        closeAudio()
    }

    companion object {
        private const val TAG = "PcrAudio"
        const val RATE = 48_000
        private const val PRIME_FRAMES = RATE * 30 / 1000
        private const val BYTES_PER_MS = RATE / 1000 * 4

        /** Keep the platform's allocation minimum; prime 30 ms before playback. */
        fun bufferBytes(min: Int) = maxOf(min, 30 * BYTES_PER_MS) // the minimum wins over the 60 ms cap: a smaller track stutters

        /** RFC 7845 identification header: stereo, 48 kHz, no pre-skip, mapping family 0. */
        fun opusHead(): ByteArray = ByteBuffer.allocate(19).order(ByteOrder.LITTLE_ENDIAN)
            .put("OpusHead".toByteArray()).put(1).put(2).putShort(0).putInt(RATE).putShort(0).put(0).array()
    }
}
