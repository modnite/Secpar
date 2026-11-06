package com.pcremote.prs
import org.junit.Assert.*
import org.junit.Test
class PcmFadeTest {
    private fun tone() = ByteArray(1920).also { b -> for (i in 0 until 480) { b[i*4]=0x10; b[i*4+1]=0x27; b[i*4+2]=0xf0.toByte(); b[i*4+3]=0xd8.toByte() } }
    @Test fun fadeStartsAtSilenceAndLeavesTheRestOfTheWaveformUntouched() {
        val b=tone();PcmFade.bridge(b,b.size)
        assertEquals(0,PcmFade.sample(b,0));assertEquals(0,PcmFade.sample(b,2))
        assertEquals(5000,PcmFade.sample(b,120*4));assertEquals(-5000,PcmFade.sample(b,120*4+2))
        assertEquals(10000,PcmFade.sample(b,240*4));assertEquals(-10000,PcmFade.sample(b,240*4+2))
    }
    @Test fun gapTransitionStartsAtThePreviousStereoSample() {
        val b=tone();PcmFade.bridge(b,b.size,2000,-2000)
        assertEquals(2000,PcmFade.sample(b,0));assertEquals(-2000,PcmFade.sample(b,2))
        assertEquals(10000,PcmFade.sample(b,240*4))
    }
}
