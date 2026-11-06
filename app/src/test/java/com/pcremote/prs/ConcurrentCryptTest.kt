package com.pcremote.prs

import org.junit.Assert.assertEquals
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicInteger
import kotlin.concurrent.thread

class ConcurrentCryptTest {
    @Test fun simultaneousVideoReceptionAndInputSendingKeepEveryPacketAuthentic() {
        val key = ByteArray(32) { it.toByte() }
        val session = 42
        val producer = Crypt(key, session)
        val shared = Crypt(key, session)
        val verifier = Crypt(key, session)
        val payload = ByteArray(256) { (it * 7).toByte() }
        val packets = Array(20_000) { i -> producer.seal(Crypt.HOST_TO_PHONE,
            Header(Prs.VIDEO, seq = i + 100_000), payload) }
        val start = CountDownLatch(1)
        val failed = AtomicInteger()
        val receive = thread {
            start.await()
            for (packet in packets) {
                if (shared.open(Crypt.HOST_TO_PHONE, packet)?.second?.contentEquals(payload) != true) failed.incrementAndGet()
            }
        }
        val transmit = thread {
            start.await()
            repeat(packets.size) { i ->
                try {
                    val packet = shared.seal(Crypt.PHONE_TO_HOST, Header(Prs.INPUT, seq = i), payload)
                    if (verifier.open(Crypt.PHONE_TO_HOST, packet)?.second?.contentEquals(payload) != true) failed.incrementAndGet()
                } catch (_: Exception) { failed.incrementAndGet() }
            }
        }
        start.countDown()
        receive.join(); transmit.join()
        assertEquals("Full-duplex PRS must not corrupt nonces", 0, failed.get())
    }
}
