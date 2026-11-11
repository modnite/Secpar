package com.pcremote.prs

/** Deadlines follow capture time, not the number of packets that survived a stall. */
internal object AudioPlayout {
    const val MAX_LATENCY_US = 80_000L
    // Packet transit age and queued output are separate limits. Adding them
    // flushes a healthy reserve during jitter and can prevent re-priming.
    const val MAX_PACKET_AGE_US = 160_000L
    data class Decision(val reset: Boolean, val discard: Boolean)
    fun decide(captureUs: Long?, nowUs: Long, queuedFrames: Long): Decision {
        val age = captureUs?.let { (nowUs - it).coerceAtLeast(0) }
        val stale = age != null && age > MAX_PACKET_AGE_US
        val queuedUs = queuedFrames.coerceAtLeast(0) * 1_000_000 / AudioPlayer.RATE
        return Decision(stale || queuedUs > MAX_LATENCY_US, stale)
    }
}
