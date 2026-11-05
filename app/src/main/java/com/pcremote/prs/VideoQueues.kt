package com.pcremote.prs

import java.util.ArrayDeque
import kotlin.math.ceil

/** Encoded P frames cannot be skipped independently: resume only at a fresh keyframe. */
internal class VideoInputQueue(fps: Int) {
    private val frames = ArrayDeque<Frame>()
    private val budgetUs = maxOf(50_000L, 2_000_000L / fps.coerceAtLeast(1))
    private val capacity = ceil(fps.coerceAtLeast(1) * budgetUs / 1_000_000.0).toInt().coerceIn(2, 8)
    private var waitingForKey = true
    var resyncs = 0; private set
    private var lastRequestUs = Long.MIN_VALUE / 2
    data class Admission(val accepted: Boolean, val requestKey: Boolean)

    fun offer(frame: Frame, nowUs: Long): Admission {
        if (frame.header.keyframe) {
            // This is a safe point to discard older compressed pictures.
            frames.clear()
            waitingForKey = false
        } else if (waitingForKey) return Admission(false, request(nowUs))
        if (frames.size >= capacity || frames.peekFirst()?.let { nowUs - it.receivedUs > budgetUs } == true) {
            resyncs++
            frames.clear()
            waitingForKey = true
            return Admission(false, request(nowUs))
        }
        frames.addLast(frame)
        return Admission(true, false)
    }
    fun poll(nowUs: Long): Frame? {
        if (frames.peekFirst()?.let { nowUs - it.receivedUs > budgetUs } == true) {
            resyncs++
            frames.clear()
            waitingForKey = true
            return null
        }
        return frames.pollFirst()
    }
    fun needsKey(nowUs: Long): Boolean = waitingForKey && request(nowUs)
    fun invalidate(nowUs: Long): Boolean {
        if (!waitingForKey) resyncs++
        frames.clear(); waitingForKey = true; return request(nowUs)
    }
    fun clear() { frames.clear() }
    private fun request(nowUs: Long): Boolean {
        if (nowUs - lastRequestUs < 250_000) return false
        lastRequestUs = nowUs
        return true
    }
}

/** Decoded pictures have no dependency on displayed pictures. Keep at most one extra for jitter. */
internal class VideoOutputQueue(fps: Int) {
    data class Picture(val index: Int, val receivedUs: Long)
    private val pictures = ArrayDeque<Picture>()
    private val minSpacingNs = 800_000_000L / fps.coerceAtLeast(1)
    private var lastRenderNs: Long? = null
    fun offer(picture: Picture): Picture? {
        val dropped = if (pictures.size == 2) pictures.removeFirst() else null
        pictures.addLast(picture)
        return dropped
    }
    fun poll(vsyncNs: Long): Picture? {
        if (lastRenderNs?.let { vsyncNs - it < minSpacingNs } == true) return null
        val picture = pictures.pollFirst() ?: return null
        lastRenderNs = vsyncNs
        return picture
    }
    fun clear(): List<Picture> = pictures.toList().also { pictures.clear() }
    fun isEmpty() = pictures.isEmpty()
}
