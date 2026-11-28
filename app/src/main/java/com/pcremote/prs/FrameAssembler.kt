package com.pcremote.prs

/** A complete encoded video frame, ready for the decoder. */
class Frame(val id: Int, val header: FrameHeader, val data: ByteArray, val receivedUs: Long, val firstUs: Long = receivedUs)

/**
 * Collects VIDEO/FEC shards into frames, in order. A frame FEC can't rebuild is asked for again
 * ([onNack]: the frame id and the missing fragment indices) once a later frame shows the gap, and
 * once more about a round trip later; later frames wait behind it. Only when the repair window
 * passes is it dropped; then non-key frames are held back (they would decode into smears) and a
 * keyframe is requested, at most once per [idrGapUs], until one arrives.
 * Not thread-safe: feed it from one thread.
 */
class FrameAssembler(
    private val onFrame: (Frame) -> Unit,
    private val onIdr: () -> Unit,
    private val onNack: (Int, IntArray) -> Unit = { _, _ -> },
    private val idrGapUs: Long = 250_000,
) {
    private class Pending(val k: Int, val p: Int, val shards: Array<ByteArray?>, val firstUs: Long) {
        val blocks = Prs.layout(k, p)
        val have = IntArray(blocks.size)
        val blockOf = IntArray(shards.size).also { arr ->
            var at = 0
            blocks.forEachIndexed { b, (kb, mb) -> for (i in 0 until kb + mb) arr[at + i] = b; at += kb + mb }
        }
        var nacks = 0
        var nackedUs = 0L
        fun ready() = blocks.indices.all { have[it] >= blocks[it].first }
        /** Missing fragments of the blocks FEC can't rebuild yet. */
        fun missing(): IntArray {
            val out = ArrayList<Int>()
            var at = 0
            blocks.forEachIndexed { b, (kb, mb) ->
                if (have[b] < kb) for (i in at until at + kb + mb) if (shards[i] == null) out += i
                at += kb + mb
            }
            return out.toIntArray()
        }
    }

    private val pending = HashMap<Int, Pending>()
    /** Complete frames waiting for an older incomplete one. */
    private val held = ArrayList<Frame>()
    private var lastDone = -1 // newest frame delivered or dropped; older shards are ignored
    private var started = false
    private var newest = 0 // newest frame any shard belonged to
    private var seen = false
    private var needKey = false
    private var lastIdrUs = Long.MIN_VALUE / 2

    /** Round trip to the host, for when to ask again and when to give up. */
    var rttUs = 50_000L

    var dropped = 0; private set
    var delivered = 0; private set

    /** How long a frame may wait for lost packets: two round trips and a margin. */
    private fun windowUs() = (2 * rttUs + 40_000).coerceIn(80_000, 300_000)

    fun onShard(h: Header, data: ByteArray, nowUs: Long) {
        if (h.typ != Prs.VIDEO && h.typ != Prs.FEC || data.size != Prs.SHARD) return
        if (started && h.frameId - lastDone <= 0) return
        if (held.any { it.id == h.frameId }) return
        val f = pending[h.frameId] ?: run {
            val n = Prs.layout(h.k, h.flags).sumOf { it.first + it.second }
            if (h.k == 0 || h.count != n || n > 4096) return
            if (pending.size >= MAX_PENDING) {
                pending.keys.minWithOrNull { a, b -> a - b }?.let { pending.remove(it); giveUp(it, nowUs) }
                release(nowUs)
                if (started && h.frameId - lastDone <= 0) return
            }
            Pending(h.k, h.flags, arrayOfNulls(n), nowUs).also { pending[h.frameId] = it }
        }
        if (!seen || h.frameId - newest > 0) { newest = h.frameId; seen = true }
        if (h.k != f.k || h.flags != f.p || h.frag >= f.shards.size || f.shards[h.frag] != null) return
        f.shards[h.frag] = data
        f.have[f.blockOf[h.frag]]++
        if (f.ready()) {
            pending.remove(h.frameId)
            val built = Frames.assemble(f.k, f.p, f.shards)
            if (built == null) giveUp(h.frameId, nowUs) else held += Frame(h.frameId, built.first, built.second, nowUs, f.firstUs)
            release(nowUs)
        }
        nack(nowUs)
    }

    /** Call every few ms: asks again for lost packets, expires frames past the repair window and
     *  repeats the keyframe request while waiting. */
    fun tick(nowUs: Long) {
        val stale = pending.filter { nowUs - it.value.firstUs > windowUs() }.keys.sortedWith { a, b -> a - b }
        for (id in stale) { pending.remove(id); giveUp(id, nowUs) }
        release(nowUs)
        nack(nowUs)
        if (needKey) idr(nowUs)
    }

    private fun nack(nowUs: Long) {
        val again = maxOf(rttUs * 3 / 2, 20_000)
        for ((id, f) in pending) {
            val age = nowUs - f.firstUs
            val gap = newest - id > 0 || age > 20_000
            val due = if (f.nacks == 0) gap && age >= NACK_WAIT_US else f.nacks < MAX_NACKS && nowUs - f.nackedUs >= again
            if (!due) continue
            val missing = f.missing()
            if (missing.isEmpty()) continue
            f.nacks++; f.nackedUs = nowUs
            onNack(id, missing)
        }
    }

    /** Hands out complete frames that no older incomplete frame blocks, in order. */
    private fun release(nowUs: Long) {
        held.sortWith { a, b -> a.id - b.id }
        while (held.isNotEmpty()) {
            val first = held[0]
            if (pending.keys.any { first.id - it > 0 }) break
            // A frame between the last one handed out and this one sent no packet at all: lost whole
            // (a small frame is one or two packets), or skipped by a host whose sending to this
            // device fell behind. This one would decode against a picture never received and smear
            // until the next keyframe. Wait a moment for it, then count it lost.
            if (started && first.id - lastDone > 1) {
                if (!needKey && !first.header.keyframe && nowUs - first.firstUs < GAP_WAIT_US) break
                dropped += first.id - lastDone - 1
                needKey = true
            }
            held.removeAt(0)
            if (started && first.id - lastDone <= 0) continue // an older drop already moved past it
            lastDone = first.id; started = true
            if (needKey && !first.header.keyframe) { idr(nowUs); continue }
            needKey = false
            delivered++
            onFrame(first)
        }
    }

    private fun giveUp(id: Int, nowUs: Long) {
        if (!started || id - lastDone > 0) { lastDone = id; started = true }
        dropped++; needKey = true; idr(nowUs)
    }

    private fun idr(nowUs: Long) {
        if (nowUs - lastIdrUs < idrGapUs) return
        lastIdrUs = nowUs
        onIdr()
    }

    private companion object {
        const val MAX_PENDING = 12
        /** How long a frame waits for one before it that sent no packet at all. */
        const val GAP_WAIT_US = 30_000L
        /** Waiting for a frame's shards before asking again: reordering on Wi-Fi. */
        const val NACK_WAIT_US = 3_000L
        const val MAX_NACKS = 2
    }
}
