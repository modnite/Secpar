package com.pcremote.prs

/** Five-millisecond stereo transition; steady audio is left untouched. */
internal object PcmFade {
    fun sample(data: ByteArray, at: Int): Int = ((data[at].toInt() and 255) or (data[at + 1].toInt() shl 8)).toShort().toInt()
    fun bridge(data: ByteArray, size: Int, left: Int = 0, right: Int = 0) {
        val count = minOf(size / 4, AudioPlayer.RATE / 200)
        for (frame in 0 until count) for (channel in 0..1) {
            val at = frame * 4 + channel * 2
            val previous = if (channel == 0) left else right
            val value = (sample(data, at) * frame + previous * (count - frame)) / count
            data[at] = value.toByte(); data[at + 1] = (value shr 8).toByte()
        }
    }
}
