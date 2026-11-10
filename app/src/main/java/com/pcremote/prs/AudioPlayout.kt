package com.pcremote.prs

/** Deadlines follow capture time, not the number of packets that survived a stall. */
internal object AudioPlayout {
    const val MAX_LATENCY_US = 80_000L
    data class Decision(val reset: Boolean, val discard: Boolean)
    // An underrun counter describes the past; do not flush a reserve already refilled.
    fun needsReprime(queuedFrames: Long) = queuedFrames < AudioPlayer.RATE * 30 / 1000
    fun decide(captureUs: Long?, nowUs: Long, queuedFrames: Long, underrun: Boolean): Decision {
        val age = captureUs?.let { (nowUs - it).coerceAtLeast(0) }
        val stale = age != null && age > MAX_LATENCY_US
        val queuedUs = queuedFrames.coerceAtLeast(0) * 1_000_000 / AudioPlayer.RATE
        return Decision((underrun && needsReprime(queuedFrames)) || stale || (age ?: 0) + queuedUs > MAX_LATENCY_US, stale)
    }
}
