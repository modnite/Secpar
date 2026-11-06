package com.pcremote.scan

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.pcremote.prs.AudioPacket
import com.pcremote.prs.AudioPlayer
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.DataInputStream

/** Real Android Opus decoder/AudioTrack, synthetic tone only; emulator has no audio output. */
@RunWith(AndroidJUnit4::class)
class AudioPlaybackInstrumentedTest {
    @Test fun batchedAudioPreservesEveryDecodedSample() {
        val packets=mutableListOf<ByteArray>()
        DataInputStream(InstrumentationRegistry.getInstrumentation().context.assets.open("audio-10ms.opuspackets")).use { input ->
            while (input.available()>0) { val n=input.readUnsignedShort(); packets.add(ByteArray(n).also(input::readFully)) }
        }
        val player=AudioPlayer { null }
        try {
            val start=System.nanoTime()
            for (i in 0 until 300) {
                // WASAPI sends audio in small batches rather than exactly every 10 ms.
                val due=start+(i/2)*20_000_000L
                val wait=due-System.nanoTime()
                if (wait>0) Thread.sleep(wait/1_000_000,(wait%1_000_000).toInt())
                player.play(AudioPacket(i,start/1000+i*10_000L,packets[i%packets.size]))
            }
            val deadline=System.nanoTime()+3_000_000_000
            while (player.acceptedFrames<300*480L && System.nanoTime()<deadline) Thread.sleep(10)
            assertEquals("No dropped or invented PCM frames",300*480L,player.acceptedFrames)
        } finally { player.release() }
    }
    @Test fun starvationAndDelayedPacketsRecoverToLiveCaptureTime() {
        val packets=mutableListOf<ByteArray>()
        DataInputStream(InstrumentationRegistry.getInstrumentation().context.assets.open("audio-10ms.opuspackets")).use { input ->
            while(input.available()>0) {val n=input.readUnsignedShort();packets.add(ByteArray(n).also(input::readFully))}
        }
        val player=AudioPlayer()
        var id=0
        fun fresh(count:Int) {
            repeat(count/2) {
                val capture=System.nanoTime()/1000
                repeat(2) { part -> player.play(AudioPacket(id++,capture+part*10_000,packets[id%packets.size])) }
                Thread.sleep(20)
            }
        }
        try {
            fresh(60)
            Thread.sleep(180) // an actual AudioTrack underrun, not a mocked counter
            repeat(10) {player.play(AudioPacket(id++,System.nanoTime()/1000-200_000,packets[id%packets.size]))}
            Thread.sleep(50)
            val before=player.acceptedFrames
            fresh(60)
            Thread.sleep(15)
            assertTrue("Output was reset after starvation",player.recoveries>0)
            assertTrue("Delayed audio was rejected",player.staleFrames>=10)
            assertTrue("Fresh audio resumes",player.acceptedFrames>before+40*480)
            assertTrue("No permanent capture lag",player.lastQueuedAgeUs<=50_000)
        } finally {player.release()}
    }

}
