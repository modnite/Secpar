package com.pcremote.prs
import org.junit.Assert.*
import org.junit.Test
class AudioPlayoutTest {
    @Test fun normalBatchesRetainTheirSmallReserve() {
        assertEquals(AudioPlayout.Decision(false,false), AudioPlayout.decide(990_000,1_000_000,1440,false))
    }
    @Test fun aStallCannotShiftTheAudioTimelinePermanently() {
        assertEquals(AudioPlayout.Decision(true,true),AudioPlayout.decide(800_000,1_000_000,3360,false))
        assertEquals(AudioPlayout.Decision(false,false),AudioPlayout.decide(1_000_000,1_005_000,0,false))
    }
    @Test fun accumulatedOutputDelayIsFlushedEvenWithFreshPackets() {
        assertEquals(AudioPlayout.Decision(true,false),AudioPlayout.decide(990_000,1_000_000,4800,false))
    }
    @Test fun underrunRequestsReprimingInsteadOfHoldingOldAudio() {
        assertTrue(AudioPlayout.decide(999_000,1_000_000,0,true).reset)
    }
    @Test fun unsynchronizedHostClockDoesNotInvalidateEveryPacket() {
        assertFalse(AudioPlayout.decide(null,1_000_000,1440,false).discard)
        assertTrue(AudioPlayout.decide(null,1_000_000,4800,false).reset)
    }
}
