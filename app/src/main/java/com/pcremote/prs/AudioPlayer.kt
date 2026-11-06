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
class AudioPlayer {
    private val exec = Executors.newSingleThreadScheduledExecutor { task -> Thread({
        android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_AUDIO)
        task.run()
    }, "pcr-audio") }
    private var codec: MediaCodec? = null
    private var pump: java.util.concurrent.ScheduledFuture<*>? = null
    @Volatile private var track: AudioTrack? = null
    private val writer = PcmWriter()
    private val head = PlaybackHead()
    private var playing = false
    private var nextPtsUs = 0L
    private var statsAt = System.nanoTime()
    private var inputWaits = 0
    private val pending = java.util.concurrent.atomic.AtomicInteger()
    @Volatile internal var acceptedFrames = 0L; private set
    private val info = MediaCodec.BufferInfo()
    private var pcm = ByteArray(0)
    @Volatile private var dead = false

    fun play(a: AudioPacket) {
        if (dead) return
        pending.incrementAndGet()
        runCatching { exec.execute {
            try { runCatching { decode(a.opus) }.onFailure { Log.w(TAG, "audio", it); shutdown() } }
            finally { pending.decrementAndGet() }
        } }.onFailure { pending.decrementAndGet() }
    }

    fun release() {
        dead = true
        // Pause wakes a blocking write, so release cannot wait behind output backpressure.
        runCatching { track?.pause() }
        exec.execute { shutdown() }
        exec.shutdown()
    }

    private fun start() {
        val fmt = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_OPUS, RATE, 2).apply {
            setInteger(MediaFormat.KEY_PCM_ENCODING, AudioFormat.ENCODING_PCM_16BIT)
            setByteBuffer("csd-0", ByteBuffer.wrap(opusHead()))
            setByteBuffer("csd-1", ByteBuffer.allocate(8).order(ByteOrder.nativeOrder()).putLong(0).flip() as ByteBuffer)
            setByteBuffer("csd-2", ByteBuffer.allocate(8).order(ByteOrder.nativeOrder()).putLong(0).flip() as ByteBuffer)
        }
        codec = MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_AUDIO_OPUS).apply { configure(fmt, null, null, 0); start() }
        val min = AudioTrack.getMinBufferSize(RATE, AudioFormat.CHANNEL_OUT_STEREO, AudioFormat.ENCODING_PCM_16BIT)
        track = AudioTrack.Builder()
            .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_GAME).setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build())
            .setAudioFormat(AudioFormat.Builder().setSampleRate(RATE).setChannelMask(AudioFormat.CHANNEL_OUT_STEREO).setEncoding(AudioFormat.ENCODING_PCM_16BIT).build())
            .setBufferSizeInBytes(bufferBytes(min))
            .setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY)
            .setTransferMode(AudioTrack.MODE_STREAM)
            .build().apply {
                if (android.os.Build.VERSION.SDK_INT >= 31) setStartThresholdInFrames(PRIME_FRAMES)
            }
        Log.i(TAG, "start bufferFrames=${track!!.bufferSizeInFrames} primeFrames=$PRIME_FRAMES")
        pump = exec.scheduleWithFixedDelay({
            if (!dead) runCatching { drain(codec!!, track!!, 0) }
                .onFailure { Log.w(TAG, "audio output", it); shutdown() }
        }, 5, 5, java.util.concurrent.TimeUnit.MILLISECONDS)
    }

    private fun decode(opus: ByteArray) {
        if (dead) return
        if (codec == null) start()
        val c = codec!!
        val t = track!!
        // Free decoded output before waiting for another input slot. Never silently
        // discard an Opus packet because the asynchronous decoder is briefly busy.
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
        c.queueInputBuffer(inIdx, 0, opus.size, nextPtsUs, 0)
        nextPtsUs += 10_000
        drain(c, t, 1_000)
        if (System.nanoTime() - statsAt >= 5_000_000_000) {
            val queued = (writer.writtenFrames - head.frames(t.playbackHeadPosition)).coerceAtLeast(0)
            Log.i(TAG, "queuedMs=${queued * 1000 / RATE} underruns=${t.underrunCount} shortWrites=${writer.shortWrites} zeroWrites=${writer.zeroWrites} inputWaits=$inputWaits pendingPackets=${pending.get()}")
            statsAt = System.nanoTime()
        }
    }

    private fun drain(c: MediaCodec, t: AudioTrack, timeoutUs: Long) {
        while (!dead) {
            val outIdx = c.dequeueOutputBuffer(info, timeoutUs)
            if (outIdx == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) continue
            if (outIdx < 0) break
            try {
                val buf = c.getOutputBuffer(outIdx)!!
                if (pcm.size < info.size) pcm = ByteArray(info.size)
                buf.position(info.offset); buf.get(pcm, 0, info.size)
                // Preserve the waveform: backpressure must not delete samples or
                // count a short/non-accepted write as a complete 10 ms packet.
                while (!dead && playing) {
                    val played = head.frames(t.playbackHeadPosition)
                    val queued = writer.writtenFrames - played
                    // Older Android devices may need their full buffer to start.
                    val limit = if (played == 0L) maxOf(RATE * 60 / 1000, t.bufferSizeInFrames) else RATE * 60 / 1000
                    if (queued <= limit) break
                    Thread.sleep(1)
                }
                writer.write(pcm, info.size, { !dead }, { Thread.sleep(1) }) { data, offset, size ->
                    t.write(data, offset, size, AudioTrack.WRITE_BLOCKING)
                }
                acceptedFrames = writer.writtenFrames
                if (!playing && !dead && writer.writtenFrames >= PRIME_FRAMES) {
                    t.play()
                    playing = true
                }
            } finally { c.releaseOutputBuffer(outIdx, false) }
        }
    }

    private fun shutdown() {
        dead = true
        pump?.cancel(false); pump = null
        runCatching { track?.stop() }; runCatching { track?.release() }; track = null
        runCatching { codec?.stop() }; runCatching { codec?.release() }; codec = null
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
