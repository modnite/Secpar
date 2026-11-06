package com.pcremote.prs

import org.junit.Assert.*
import org.junit.Test

class PcmTimelineTest {
    @Test fun resumedAudioAndDroppedPacketsKeepTheirRealCaptureTime() {
        val timeline=PcmTimeline()
        timeline.queued(1_000_000); timeline.queued(1_010_000)
        assertEquals(1_000_000L,timeline.consume(480))
        assertEquals(1_010_000L,timeline.consume(480))
        // A decoder's sample-count clock would say 1,020,000 here forever,
        // despite the capture clock advancing across the playback pause.
        timeline.queued(6_000_000);timeline.queued(6_030_000)
        assertEquals(6_000_000L,timeline.consume(480))
        assertEquals(6_030_000L,timeline.consume(480))
    }
    @Test fun splitAndCombinedOutputBuffersPreserveSamplePositions() {
        val timeline=PcmTimeline()
        timeline.queued(1_000_000);timeline.queued(1_010_000);timeline.queued(1_020_000)
        assertEquals(1_000_000L,timeline.consume(240))
        assertEquals(1_005_000L,timeline.consume(720))
        assertEquals(1_020_000L,timeline.consume(480))
        assertNull(timeline.consume(480))
    }
    @Test fun anUnexpectedOutputCannotShiftEveryLaterPacket() {
        val timeline=PcmTimeline();timeline.queued(1_000_000)
        assertNull(timeline.consume(960))
        timeline.queued(2_000_000)
        assertEquals(2_000_000L,timeline.consume(480))
    }
}
