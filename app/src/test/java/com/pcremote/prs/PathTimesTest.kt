package com.pcremote.prs

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PathTimesTest {
    @Test fun `the other path wins only when clearly faster over five samples each`() {
        val p = PathTimes()
        repeat(5) { p.current(80) }
        repeat(4) { p.other(30) }
        assertFalse("needs five samples of each", p.otherFaster())
        p.other(30)
        assertTrue(p.otherFaster())
    }

    @Test fun `small or relative-only gains keep the current path`() {
        val p = PathTimes()
        repeat(5) { p.current(20); p.other(10) } // half the time, but only 10 ms
        assertFalse(p.otherFaster())
        val q = PathTimes()
        repeat(5) { q.current(100); q.other(75) } // 25 ms, but not 30 % faster
        assertFalse(q.otherFaster())
    }
}
