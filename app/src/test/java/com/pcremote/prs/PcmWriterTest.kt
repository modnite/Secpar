package com.pcremote.prs

import org.junit.Assert.*
import org.junit.Test
import java.io.ByteArrayOutputStream

class PcmWriterTest {
    @Test fun shortAndZeroWritesPreserveEverySampleAndCountOnlyAcceptedBytes() {
        val writer = PcmWriter()
        val source = ByteArray(1920) { it.toByte() }
        val output = ByteArrayOutputStream()
        val returns = ArrayDeque(listOf(400, 0, 800, 720))
        var waits = 0
        writer.write(source, source.size, { true }, { waits++ }) { data, offset, remaining ->
            assertEquals(offset.toLong(), writer.writtenBytes)
            val accepted = returns.removeFirst()
            assertTrue(accepted <= remaining)
            output.write(data, offset, accepted)
            accepted
        }
        assertArrayEquals(source, output.toByteArray())
        assertEquals(480L, writer.writtenFrames)
        assertEquals(2, writer.shortWrites)
        assertEquals(1, waits)
    }
    @Test fun stoppingDoesNotInventUnwrittenFrames() {
        val writer = PcmWriter(); var alive = true
        writer.write(ByteArray(1920),1920,{alive},{}) { _,_,_ -> alive=false; 400 }
        assertEquals(100L,writer.writtenFrames)
    }
    @Test fun outputErrorsNeverIncreaseAcceptedCount() {
        val writer=PcmWriter()
        try { writer.write(ByteArray(1920),1920,{true},{}) { _,_,_ -> -6 }; fail("must reject failed write") }
        catch (_: IllegalStateException) { assertEquals(0L,writer.writtenFrames) }
    }
    @Test fun playbackHeadUsesUnsignedValuesAndWraps() {
        val head=PlaybackHead()
        assertEquals(0x7fff_ffffL,head.frames(Int.MAX_VALUE))
        assertEquals(0xffff_fff0L,head.frames(-16))
        assertEquals(0x1_0000_0010L,head.frames(16))
    }
}
