package com.pcremote.prs

/** Deadlines follow capture time, not the number of packets that survived a stall. */
internal object AudioPlayout {
    const val MAX_LATENCY_US = 80_000L
    // Packet transit age and queued output are separate limits. Adding them
    // flushes a healthy reserve during jitter and can prevent re-priming.
    // Only audio from a real stall is discarded: Wi-Fi on phones and TVs holds packets for 200-300 ms
    // at times, and discarding them (and resetting the output) cut out a second or two where the
    // desktop client, which only bounds its queue, played on. The queue limits below keep the delay.
    const val MAX_PACKET_AGE_US = 400_000L
    data class Decision(val reset: Boolean, val discard: Boolean)
    fun decide(captureUs: Long?, nowUs: Long, queuedFrames: Long): Decision {
        val age = captureUs?.let { (nowUs - it).coerceAtLeast(0) }
        val stale = age != null && age > MAX_PACKET_AGE_US
        val queuedUs = queuedFrames.coerceAtLeast(0) * 1_000_000 / AudioPlayer.RATE
        return Decision(stale || queuedUs > MAX_LATENCY_US, stale)
    }
}
