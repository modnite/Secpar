package com.pcremote.prs

import org.junit.Assert.*
import org.junit.Test

class VideoQueuesTest {
    private fun frame(id: Int, key: Boolean = false, time: Long = 1_000L) =
        Frame(id, FrameHeader(1, time, key, 1), byteArrayOf(0), time)

    @Test fun overflowWaitsForAKeyframeAndDoesNotFeedBrokenDependencies() {
        val queue = VideoInputQueue(60)
        assertTrue(queue.offer(frame(0, true), 1_000).accepted)
        assertTrue(queue.offer(frame(1), 2_000).accepted)
        assertTrue(queue.offer(frame(2), 3_000).accepted)
        val overflow = queue.offer(frame(3), 4_000)
        assertFalse(overflow.accepted); assertTrue(overflow.requestKey)
        assertNull(queue.poll(5_000))
        val dependent = queue.offer(frame(4), 6_000)
        assertFalse(dependent.accepted); assertFalse(dependent.requestKey)
        assertTrue(queue.offer(frame(5, true, 7_000), 7_000).accepted)
        assertEquals(5, queue.poll(8_000)?.id)
    }

    @Test fun stalledInputIsExpiredEvenWithOnlyOneQueuedFrame() {
        val queue = VideoInputQueue(120)
        queue.offer(frame(0, true), 1_000)
        assertNull(queue.poll(80_000))
        assertTrue(queue.needsKey(80_000))
        assertFalse(queue.offer(frame(1, time = 81_000), 81_000).accepted)
    }

    @Test fun recoveryRequestsAreRetriedButRateLimited() {
        val queue = VideoInputQueue(60)
        assertTrue(queue.offer(frame(1), 1_000).requestKey)
        assertFalse(queue.offer(frame(2), 250_999).requestKey)
        assertTrue(queue.offer(frame(3), 251_000).requestKey)
    }

    @Test fun aNewKeyframeSafelyReplacesQueuedOldPictures() {
        val queue = VideoInputQueue(60)
        queue.offer(frame(0, true), 1_000)
        queue.offer(frame(1), 2_000)
        queue.offer(frame(2, true, 3_000), 3_000)
        assertEquals(2, queue.poll(4_000)?.id)
        assertNull(queue.poll(4_000))
    }

    @Test fun outputBurstRetainsOnlyTwoPicturesAndReleasesTheOlderLease() {
        val queue = VideoOutputQueue(120)
        assertNull(queue.offer(VideoOutputQueue.Picture(0, 1)))
        assertNull(queue.offer(VideoOutputQueue.Picture(1, 2)))
        assertEquals(0, queue.offer(VideoOutputQueue.Picture(2, 3))?.index)
        assertEquals(1, queue.poll(0)?.index)
        assertEquals(2, queue.poll(16_666_667)?.index)
        assertTrue(queue.isEmpty())
    }

    @Test fun sixtyFpsOnA120HzDisplayDoesNotPresentTwiceInAdjacentRefreshes() {
        val queue = VideoOutputQueue(60)
        queue.offer(VideoOutputQueue.Picture(0, 1)); queue.offer(VideoOutputQueue.Picture(1, 2))
        assertEquals(0, queue.poll(0)?.index)
        assertNull(queue.poll(8_333_333))
        assertEquals(1, queue.poll(16_666_667)?.index)
    }

    @Test fun a60HzDisplayNeverPlaysBackA120FpsBurstInBetweenRefreshes() {
        val queue = VideoOutputQueue(120)
        for (frame in 0..29) queue.offer(VideoOutputQueue.Picture(frame, frame.toLong()))
        assertEquals(28, queue.poll(1_000_000_000)?.index)
        assertEquals(29, queue.poll(1_016_666_667)?.index)
        assertNull(queue.poll(1_033_333_334))
    }

    @Test fun teardownReturnsEveryOutstandingOutputBuffer() {
        val queue = VideoOutputQueue(60)
        queue.offer(VideoOutputQueue.Picture(4, 1)); queue.offer(VideoOutputQueue.Picture(9, 2))
        assertEquals(listOf(4, 9), queue.clear().map { it.index })
        assertTrue(queue.isEmpty()); assertTrue(queue.clear().isEmpty())
    }
}
