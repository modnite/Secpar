package com.pcremote.prs

/** Deadlines follow capture time, not the number of packets that survived a stall. */
internal object AudioPlayout {
    const val MAX_AGE_US = 40_000L
    const val MAX_LATENCY_US = 80_000L
    data class Decision(val reset: Boolean, val discard: Boolean)
    fun decide(captureUs: Long?, nowUs: Long, queuedFrames: Long, underrun: Boolean): Decision {
        val age = captureUs?.let { (nowUs - it).coerceAtLeast(0) }
        val stale = age != null && age > MAX_AGE_US
        val queuedUs = queuedFrames.coerceAtLeast(0) * 1_000_000 / AudioPlayer.RATE
        return Decision(underrun || stale || (age ?: 0) + queuedUs > MAX_LATENCY_US, stale)
    }
}
