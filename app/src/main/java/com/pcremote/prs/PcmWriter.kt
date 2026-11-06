package com.pcremote.prs

/** Counts only accepted PCM and retries the unwritten tail of a short write. */
internal class PcmWriter {
    var writtenBytes = 0L; private set
    var shortWrites = 0; private set
    var zeroWrites = 0; private set
    val writtenFrames get() = writtenBytes / 4

    fun write(data: ByteArray, size: Int, alive: () -> Boolean, wait: () -> Unit,
              sink: (ByteArray, Int, Int) -> Int) {
        require(size in 0..data.size && size % 4 == 0)
        var offset = 0
        while (offset < size && alive()) {
            val remaining = size - offset
            val count = sink(data, offset, remaining)
            check(count in 0..remaining && count % 4 == 0) { "PCM output write failed: $count" }
            if (count == 0) { zeroWrites++; wait(); continue }
            if (count < remaining) shortWrites++
            offset += count
            writtenBytes += count
        }
    }
}

/** AudioTrack exposes an unsigned 32-bit frame count, which wraps on long sessions. */
internal class PlaybackHead {
    private var last = 0L
    private var epoch = 0L
    fun frames(raw: Int): Long {
        val now = raw.toLong() and 0xffff_ffffL
        if (now < last) epoch += 1L shl 32
        last = now
        return epoch + now
    }
}
