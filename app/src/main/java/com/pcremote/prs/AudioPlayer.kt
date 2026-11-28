package com.pcremote.prs

import android.media.AudioFormat
import android.media.AudioTrack
import android.media.MediaCodec
import android.media.MediaFormat
import android.util.Log
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.Executors

/** Opus (48 kHz stereo, 10 ms packets) → MediaCodec → low-latency output ([PcmOut]), with a 30–60 ms buffer. */
class AudioPlayer(private val captureToLocalUs: (Long) -> Long? = { it }) {
    private val exec = Executors.newSingleThreadScheduledExecutor { task -> Thread({
        android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_AUDIO)
        task.run()
    }, "pcr-audio") }
    private var codec: MediaCodec? = null
    private var pump: java.util.concurrent.ScheduledFuture<*>? = null
    @Volatile private var track: PcmOut? = null
    private var writer = PcmWriter()
    private var playing = false
    private var fading = true
    private var previousLeft = 0
    private var previousRight = 0
    private var gapCaptureUs: Long? = null
    private var firstBufferedAtUs: Long? = null
    private var lastPacketId: Int? = null
    @Volatile internal var recoveries = 0; private set
    @Volatile internal var staleFrames = 0; private set
    @Volatile internal var lastQueuedAgeUs = 0L; private set
    private var statsAt = System.nanoTime()
    private var inputWaits = 0
    private var inputStalls = 0
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
    private var trimmedPackets = 0
    internal fun diagnostics() = "submitted=$submittedPackets decoded=$decodedFrames accepted=$acceptedFrames overflow=${overflowPackets.get()} firstOverflow=[$firstOverflow] stale=$staleFrames incomplete=$incompleteWrites timelineMisses=$timelineMisses recoveries=$recoveries audioFailures=$audioFailures recoveryDrops=$recoveryDrops pending=${pending.get()} codecTimestampOffsetUs=$codecTimestampOffsetUs"
    /** Output device and counters, for the stream menu's Audio page. */
    fun details(): String {
        val output = runCatching { track?.describe() }.getOrNull()
        return "Output: ${output ?: "not open"}\n" + diagnostics()
    }

    /** Close the output and open it again on the next packet; clears the restart back-off. */
    fun reopen() {
        if (dead) return
        runCatching { exec.execute {
            if (dead) return@execute
            closeAudio()
            failureStreak = 0; retryAfterNs = 0; healthySinceNs = 0
            runCatching { start() }.onFailure { recoverAudio(it) }
        } }
    }

    private val info = MediaCodec.BufferInfo()
    private var pcm = ByteArray(0)
    @Volatile private var dead = false
    /** Muted plays silence: the output keeps its timing, so unmuting is instant and in sync. */
    @Volatile private var muted = false

    fun setMuted(on: Boolean) {
        if (muted == on) return
        muted = on
        if (!on) runCatching { exec.execute { fading = true } } // fade back in rather than click
    }

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
        runCatching { (track as? TrackOut)?.track?.pause() }
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
        // AAudio where it opens: on TVs it skips the mixer's large blocks (about 90 ms less delay).
        track = AAudioOut.open(RATE, AAUDIO_BUFFER_FRAMES) ?: TrackOut(RATE, bufferBytes(min), PRIME_FRAMES)
        Log.i(TAG, "start decoder=${codec!!.name} output=[${track!!.describe()}] primeFrames=$PRIME_FRAMES")
        pump = exec.scheduleWithFixedDelay({
            if (!dead) runCatching { drain(codec!!, track!!, 0) }
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
        drain(c, t, 0)
        val deadline = System.nanoTime() + 100_000_000
        var inIdx = c.dequeueInputBuffer(5_000)
        while (inIdx < 0 && !dead && System.nanoTime() < deadline) {
            inputWaits++
            drain(c, t, 1_000)
            inIdx = c.dequeueInputBuffer(5_000)
        }
        if (dead) return
        // A decoder busy for 100 ms loses this packet (Opus covers the gap); only one that stays
        // stuck is restarted. Restarting at once, with its back-off, silenced a second or more.
        if (inIdx < 0) {
            check(++inputStalls < 3) { "Audio decoder stopped accepting packets" }
            return
        }
        inputStalls = 0
        c.getInputBuffer(inIdx)!!.apply { clear(); put(opus) }
        c.queueInputBuffer(inIdx, 0, opus.size, packet.captureUs, 0)
        timeline.queued(packet.captureUs)
        submittedPackets++
        drain(c, t, 1_000)
        if (System.nanoTime() - statsAt >= 5_000_000_000) {
            val queued = queuedFrames(t)
            Log.i(TAG, "queuedMs=${queued * 1000 / RATE} underruns=${t.underruns()} shortWrites=${writer.shortWrites} zeroWrites=${writer.zeroWrites} inputWaits=$inputWaits pendingPackets=${pending.get()} overflowPackets=${overflowPackets.get()} recoveries=$recoveries audioFailures=$audioFailures recoveryDrops=$recoveryDrops trimmed=$trimmedPackets staleFrames=$staleFrames captureAgeMs=${lastQueuedAgeUs / 1000} inputAgeMs=${inputAgeUs / 1000} decodedAgeMs=${decodedAgeUs / 1000} codecTimestampOffsetMs=${codecTimestampOffsetUs / 1000} timelineMisses=$timelineMisses output=[${t.describe()}]")
            statsAt = System.nanoTime()
        }
    }

    private fun drain(c: MediaCodec, t: PcmOut, timeoutUs: Long) {
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
                if (!playing && firstBufferedAtUs?.let { nowUs - it > AudioPlayout.MAX_LATENCY_US } == true) resetOutput(t, "startup-expired", nowUs - firstBufferedAtUs!!)
                val queued = queuedFrames(t)
                val decision = AudioPlayout.decide(captureUs, nowUs, queued)
                if (decision.reset && (playing || writer.writtenFrames > 0)) resetOutput(t, if (decision.discard) "stale-packet" else "latency-budget", decodedAgeUs, queued)
                if (decision.discard) { staleFrames++; continue }
                // A low-latency output reads at exactly the incoming rate, so a backlog from startup or a
                // network burst would stay for good. Skip a packet instead; the next one is blended in.
                if (playing && t.trims && queued > TRIM_ABOVE_FRAMES) { trimmedPackets++; gapCaptureUs = hostCaptureUs; continue }
                if (muted) java.util.Arrays.fill(pcm, 0, info.size, 0)
                if (gapCaptureUs?.let { hostCaptureUs >= it } == true) {
                    PcmFade.bridge(pcm, info.size, previousLeft, previousRight)
                    gapCaptureUs = null
                } else if (fading) PcmFade.bridge(pcm, info.size)
                fading = false
                val before = writer.writtenFrames
                // Waits a short while for room in the output, however late the packet arrived: a
                // deadline from its capture time wrote nothing for a late packet and reset the output.
                val deadlineUs = nowUs + WRITE_WAIT_US
                writer.write(pcm, info.size, { !dead && System.nanoTime() / 1000 <= deadlineUs }, { Thread.sleep(1) }) { data, offset, size ->
                    t.write(data, offset, size)
                }
                val accepted = writer.writtenFrames - before
                acceptedFrames += accepted
                if (accepted > 0) {
                    val nowNs = System.nanoTime()
                    if (healthySinceNs == 0L) healthySinceNs = nowNs
                    if (nowNs - healthySinceNs >= 1_000_000_000L) failureStreak = 0
                }
                if (!dead && playing && t.behind()) { resetOutput(t, "output-behind", decodedAgeUs); continue }
                if (!dead && accepted * 4 < info.size) { incompleteWrites++; staleFrames++; resetOutput(t, "incomplete-write", decodedAgeUs); continue }
                if (info.size >= 4) {
                    previousLeft = PcmFade.sample(pcm, info.size - 4)
                    previousRight = PcmFade.sample(pcm, info.size - 2)
                }
                if (firstBufferedAtUs == null) firstBufferedAtUs = System.nanoTime() / 1000
                lastQueuedAgeUs = captureUs?.let { (System.nanoTime() / 1000 - it).coerceAtLeast(0) } ?: 0
                if (!playing && !dead && writer.writtenFrames >= PRIME_FRAMES) {
                    t.play()
                    playing = true
                }
            } finally { c.releaseOutputBuffer(outIdx, false) }
        }
    }

    private fun queuedFrames(t: PcmOut) = if (playing) (t.queuedFrames() ?: (writer.writtenFrames - t.playedFrames())).coerceAtLeast(0) else writer.writtenFrames

    // AudioTrack resumes when fresh PCM arrives. Flushing on its cumulative
    // underrun counter can itself starve playback and start another reset cycle.
    private fun resetOutput(t: PcmOut, reason: String, captureAgeUs: Long, queued: Long = queuedFrames(t)) {
        Log.i(TAG, "output reset reason=$reason queuedMs=${queued * 1000 / RATE} captureAgeMs=${captureAgeUs / 1000} underruns=${t.underruns()} pendingPackets=${pending.get()} playing=$playing")
        t.reset()
        writer = PcmWriter()
        playing = false
        fading = true
        previousLeft = 0; previousRight = 0
        firstBufferedAtUs = null
        recoveries++
    }

    private fun closeAudio() {
        pump?.cancel(false); pump = null
        runCatching { track?.release() }; track = null
        runCatching { codec?.stop() }; runCatching { codec?.release() }; codec = null
        timeline.clear()
        writer = PcmWriter()
        playing = false
        fading = true
        previousLeft = 0; previousRight = 0
        firstBufferedAtUs = null
        gapCaptureUs = null
    }

    private fun recoverAudio(failure: Throwable) {
        closeAudio()
        if (dead) return
        audioFailures++
        failureStreak = (failureStreak + 1).coerceAtMost(6)
        val retryMs = (50L shl (failureStreak - 1)).coerceAtMost(250)
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
        /** The AAudio buffer holds the player's whole latency budget plus a margin. */
        private const val AAUDIO_BUFFER_FRAMES = RATE / 10
        /** Above this a low-latency output drops a packet: about 30 to 40 ms stays queued. */
        private const val TRIM_ABOVE_FRAMES = RATE * 40 / 1000
        /** How long a write waits for room in the output. */
        private const val WRITE_WAIT_US = 40_000L

        /** Keep the platform's allocation minimum; prime 30 ms before playback. */
        fun bufferBytes(min: Int) = maxOf(min, 30 * BYTES_PER_MS) // the minimum wins over the 60 ms cap: a smaller track stutters

        /** RFC 7845 identification header: stereo, 48 kHz, no pre-skip, mapping family 0. */
        fun opusHead(): ByteArray = ByteBuffer.allocate(19).order(ByteOrder.LITTLE_ENDIAN)
            .put("OpusHead".toByteArray()).put(1).put(2).putShort(0).putInt(RATE).putShort(0).put(0).array()
    }
}
