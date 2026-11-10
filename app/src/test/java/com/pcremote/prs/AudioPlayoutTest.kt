package com.pcremote.prs
import org.junit.Assert.*
import org.junit.Test
class AudioPlayoutTest {
    @Test fun normalBatchesRetainTheirSmallReserve() {
        assertEquals(AudioPlayout.Decision(false,false), AudioPlayout.decide(990_000,1_000_000,1440))
    }
    @Test fun aStallCannotShiftTheAudioTimelinePermanently() {
        assertEquals(AudioPlayout.Decision(true,true),AudioPlayout.decide(800_000,1_000_000,3360))
        assertEquals(AudioPlayout.Decision(false,false),AudioPlayout.decide(1_000_000,1_005_000,0))
    }
    @Test fun accumulatedOutputDelayIsFlushedEvenWithFreshPackets() {
        assertEquals(AudioPlayout.Decision(true,false),AudioPlayout.decide(990_000,1_000_000,4800))
    }
    @Test fun aDelayedBatchWithinTheLatencyBudgetIsNotDiscarded() {
        assertEquals(AudioPlayout.Decision(false,false),AudioPlayout.decide(955_000,1_000_000,960))
    }
    @Test fun overdueQueueIsFlushedButUsableIncomingAudioIsRetained() {
        assertEquals(AudioPlayout.Decision(true,false),AudioPlayout.decide(950_000,1_000_000,1920))
        assertEquals(AudioPlayout.Decision(true,true),AudioPlayout.decide(919_000,1_000_000,0))
    }
    @Test fun unsynchronizedHostClockDoesNotInvalidateEveryPacket() {
        assertFalse(AudioPlayout.decide(null,1_000_000,1440).discard)
        assertTrue(AudioPlayout.decide(null,1_000_000,4800).reset)
    }
}
