package com.pcremote.prs

/**
 * How much later than usual video frames arrive: capture (host clock) to first shard (this
 * clock), minus the smallest such gap of the last 5-10 s. The clocks' offset cancels out; a rising
 * value is a queue filling on the path, which the host's bitrate control answers before it
 * overflows into loss. Same as the desktop client's `QueueDelay`.
 */
class QueueDelay {
    private val base = arrayOfNulls<Long>(2)
    private var bucketAtUs = Long.MIN_VALUE
    private var sum = 0L
    private var n = 0L

    fun add(captureUs: Long, arrivedUs: Long) {
        if (captureUs == 0L) return
        val gap = arrivedUs - captureUs
        if (bucketAtUs == Long.MIN_VALUE) bucketAtUs = arrivedUs
        if (arrivedUs - bucketAtUs >= 5_000_000) { base[0] = base[1]; base[1] = null; bucketAtUs = arrivedUs }
        base[1] = minOf(base[1] ?: gap, gap)
        val b = listOfNotNull(base[0], base[1]).minOrNull() ?: gap
        sum += gap - b; n++
    }

    /** The average since the last call, ms (0 without frames). */
    fun takeMs(): Int {
        val ms = if (n == 0L) 0L else sum / n / 1000
        sum = 0; n = 0
        return ms.coerceIn(0, 65535).toInt()
    }
}
