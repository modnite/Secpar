package com.pcremote.prs

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.util.Log

/**
 * Where decoded stream audio is played. [AAudioOut] reaches the low-latency (MMAP) output where a
 * device has one; [TrackOut] is the AudioTrack used before, kept for devices where AAudio fails.
 * Calls never block: [write] returns how many bytes were taken.
 */
internal interface PcmOut {
    fun write(data: ByteArray, offset: Int, size: Int): Int
    fun play()
    /** Drops everything queued and waits for new audio before playing again. */
    fun reset()
    /** Frames the device has taken since this output opened or was last reset. */
    fun playedFrames(): Long
    fun underruns(): Int
    fun describe(): String
    fun release()
    /** Frames waiting in the output by its own counters, or null to count from the player's writes. */
    fun queuedFrames(): Long? = null
    /** The output reads ahead of what was written: whatever is written now has already played. */
    fun behind(): Boolean = false
    /** Whether the player trims its queue by dropping packets (outputs read at the exact rate). */
    val trims: Boolean get() = false
}

internal object AAudioNative {
    val loaded = android.os.Build.VERSION.SDK_INT >= 28 && runCatching { System.loadLibrary("secparaudio") }.isSuccess
    external fun open(rate: Int, bufferFrames: Int): Long
    external fun write(h: Long, data: ByteArray, offset: Int, bytes: Int): Int
    external fun start(h: Long): Int
    external fun framesRead(h: Long): Long
    external fun framesWritten(h: Long): Long
    external fun underruns(h: Long): Int
    external fun burst(h: Long): Int
    external fun bufferFrames(h: Long): Int
    external fun deviceId(h: Long): Int
    external fun state(h: Long): Int
    external fun latencyUs(h: Long): Long
    external fun mmapUsed(h: Long): Boolean
    external fun close(h: Long)
}

/** AAudio, low latency. A reset closes the stream and opens a new one, so its counters start at zero. */
internal class AAudioOut private constructor(private val rate: Int, private val bufferFrames: Int, private var h: Long) : PcmOut {
    private var underrunsBefore = 0
    override val trims get() = true

    override fun write(data: ByteArray, offset: Int, size: Int): Int {
        val n = AAudioNative.write(h, data, offset, size)
        check(n >= 0) { "AAudio write failed ($n)" } // disconnected (output changed): the player reopens
        return n
    }
    override fun play() { check(AAudioNative.start(h) >= 0) { "AAudio start failed" } }
    override fun reset() {
        underrunsBefore += AAudioNative.underruns(h)
        AAudioNative.close(h)
        h = AAudioNative.open(rate, bufferFrames)
        check(h != 0L && AAudioNative.mmapUsed(h)) { "AAudio reopen failed" } // the player restarts and may fall back
    }
    override fun playedFrames() = AAudioNative.framesRead(h)
    override fun queuedFrames() = AAudioNative.framesWritten(h) - AAudioNative.framesRead(h)
    // After a gap in the audio, an MMAP stream keeps reading at its own pace and does not move
    // the write position forward: every later write lands behind it, silently.
    override fun behind() = AAudioNative.framesWritten(h) < AAudioNative.framesRead(h)
    override fun underruns() = underrunsBefore + AAudioNative.underruns(h)
    override fun describe(): String {
        val latency = AAudioNative.latencyUs(h)
        return "AAudio MMAP, burst ${AAudioNative.burst(h) * 1000 / rate} ms" +
            (if (latency >= 0) ", output delay ${latency / 1000} ms" else "")
    }
    override fun release() { if (h != 0L) AAudioNative.close(h); h = 0 }

    companion object {
        fun open(rate: Int, bufferFrames: Int): AAudioOut? {
            if (!AAudioNative.loaded) return null
            val h = runCatching { AAudioNative.open(rate, bufferFrames) }.getOrDefault(0L)
            if (h == 0L) { Log.w("PcrAudio", "AAudio unavailable; using AudioTrack"); return null }
            // Without MMAP, AAudio is an AudioTrack underneath that starts only once its buffer is full.
            if (!AAudioNative.mmapUsed(h)) { AAudioNative.close(h); Log.i("PcrAudio", "no MMAP output; using AudioTrack"); return null }
            return AAudioOut(rate, bufferFrames, h)
        }
    }
}

/** The platform AudioTrack in low-latency mode; on TVs it goes through the mixer's large blocks. */
internal class TrackOut(rate: Int, bufferBytes: Int, startFrames: Int) : PcmOut {
    val track: AudioTrack = AudioTrack.Builder()
        .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_GAME).setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build())
        .setAudioFormat(AudioFormat.Builder().setSampleRate(rate).setChannelMask(AudioFormat.CHANNEL_OUT_STEREO).setEncoding(AudioFormat.ENCODING_PCM_16BIT).build())
        .setBufferSizeInBytes(bufferBytes)
        .setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY)
        .setTransferMode(AudioTrack.MODE_STREAM)
        .build()
        .also { if (android.os.Build.VERSION.SDK_INT >= 31) it.setStartThresholdInFrames(startFrames) }
    private var head = PlaybackHead()

    override fun write(data: ByteArray, offset: Int, size: Int) = track.write(data, offset, size, AudioTrack.WRITE_NON_BLOCKING)
    override fun play() = track.play()
    override fun reset() { track.pause(); track.flush(); head = PlaybackHead() }
    override fun playedFrames() = head.frames(track.playbackHeadPosition)
    override fun underruns() = track.underrunCount
    override fun describe(): String {
        val device = runCatching { track.routedDevice?.let { d -> d.productName?.toString().orEmpty().ifBlank { "device ${d.type}" } } }.getOrNull()
        return "AudioTrack, ${device ?: "no device"}, buffer ${track.bufferSizeInFrames * 1000 / track.sampleRate} ms"
    }
    override fun release() { runCatching { track.stop() }; track.release() }
}
