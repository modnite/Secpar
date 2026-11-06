package com.pcremote.prs

/** Opus output is ordered PCM. Keep capture time beside the submitted samples;
 * a decoder may synthesize a continuous output PTS across packet loss or pauses. */
internal class PcmTimeline {
    private data class Chunk(val captureUs: Long, var remaining: Int, var consumed: Int = 0)
    private val chunks = java.util.ArrayDeque<Chunk>()

    fun queued(captureUs: Long, frames: Int = 480) {
        require(frames > 0)
        check(chunks.size < 64) { "Opus timing queue exceeded decoder capacity" }
        chunks.addLast(Chunk(captureUs, frames))
    }

    fun consume(frames: Int): Long? {
        if (frames <= 0) return null
        if (chunks.sumOf { it.remaining } < frames) {
            chunks.clear()
            return null
        }
        val first = chunks.first
        val captureUs = first.captureUs + first.consumed * 1_000_000L / AudioPlayer.RATE
        var left = frames
        while (left > 0) {
            val chunk = chunks.first
            val take = minOf(left, chunk.remaining)
            chunk.remaining -= take; chunk.consumed += take; left -= take
            if (chunk.remaining == 0) chunks.removeFirst()
        }
        return captureUs
    }
}
