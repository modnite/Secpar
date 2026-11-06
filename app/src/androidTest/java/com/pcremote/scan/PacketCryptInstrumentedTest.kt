package com.pcremote.scan

import androidx.test.ext.junit.runners.AndroidJUnit4
import com.pcremote.prs.Crypt
import com.pcremote.prs.Header
import com.pcremote.prs.Prs
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicInteger
import kotlin.concurrent.thread

/** Exercises Android's actual crypto provider with simultaneous video and input. */
@RunWith(AndroidJUnit4::class)
class PacketCryptInstrumentedTest {
    @Test fun fullDuplexPacketsRemainAuthentic() {
        val key = ByteArray(32) { it.toByte() }
        val shared = Crypt(key, 42)
        val producer = Crypt(key, 42)
        val verifier = Crypt(key, 42)
        val video = ByteArray(1152) { (it * 7).toByte() }
        val input = byteArrayOf(1, 2, 3, 4)
        val packets = Array(6_000) { producer.seal(Crypt.HOST_TO_PHONE, Header(Prs.VIDEO, seq = it + 100_000), video) }
        val start = CountDownLatch(1)
        val failures = AtomicInteger()
        val receive = thread {
            start.await()
            for (packet in packets) if (shared.open(Crypt.HOST_TO_PHONE, packet)?.second?.contentEquals(video) != true) failures.incrementAndGet()
        }
        val send = thread {
            start.await()
            repeat(packets.size) {
                try {
                    val packet = shared.seal(Crypt.PHONE_TO_HOST, Header(Prs.INPUT, seq = it), input)
                    if (verifier.open(Crypt.PHONE_TO_HOST, packet)?.second?.contentEquals(input) != true) failures.incrementAndGet()
                } catch (_: Exception) { failures.incrementAndGet() }
            }
        }
        start.countDown(); receive.join(); send.join()
        assertEquals("No authenticated video/input packets may be corrupted", 0, failures.get())
    }
}
