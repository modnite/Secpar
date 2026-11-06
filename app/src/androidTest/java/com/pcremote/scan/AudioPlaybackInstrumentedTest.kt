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
    private fun tonePackets(): List<ByteArray> {
        val packets=mutableListOf<ByteArray>()
        DataInputStream(InstrumentationRegistry.getInstrumentation().context.assets.open("audio-10ms.opuspackets")).use { input ->
            while(input.available()>0) {val n=input.readUnsignedShort();packets.add(ByteArray(n).also(input::readFully))}
        }
        return packets
    }

    @Test fun batchedAudioPreservesEveryDecodedSample() {
        val packets=mutableListOf<ByteArray>()
        DataInputStream(InstrumentationRegistry.getInstrumentation().context.assets.open("audio-10ms.opuspackets")).use { input ->
            while (input.available()>0) { val n=input.readUnsignedShort(); packets.add(ByteArray(n).also(input::readFully)) }
        }
        val player=AudioPlayer { null }
        try {
            player.prepare()
            assertEquals("Preparation submits no synthetic audio",0L,player.submittedPackets)
            assertEquals("Preparation writes no synthetic PCM",0L,player.acceptedFrames)
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
            assertEquals("No dropped or invented PCM frames: ${player.diagnostics()}",300*480L,player.acceptedFrames)
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
            player.prepare()
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

    @Test fun repeatedVideoSwitchesAndLongSilenceRecoverWithoutReconnect() {
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
            player.prepare()
            fresh(60)
            for (silenceMs in listOf(250L, 1200L, 2500L)) {
                Thread.sleep(silenceMs)
                // Switching media may leave delayed packets and a sequence gap.
                repeat(6) {player.play(AudioPacket(id++,System.nanoTime()/1000-200_000,packets[id%packets.size]))}
                Thread.sleep(50)
                id+=3
                val before=player.acceptedFrames
                val staleBefore=player.staleFrames
                fresh(80)
                Thread.sleep(15)
                val details="silenceMs=$silenceMs ${player.diagnostics()}"
                assertTrue("Fresh playback resumes promptly: $details",player.acceptedFrames>=before+70*480)
                assertTrue("Fresh packets stay current: $details",player.lastQueuedAgeUs<=50_000)
                assertTrue("Fresh capture does not remain stale: $details",player.staleFrames-staleBefore<=4)
                assertEquals("Every decoded buffer retains capture mapping: $details",0,player.timelineMisses)
                assertEquals("No PCM tail lost during switches: $details",0,player.incompleteWrites)
                android.util.Log.i("PcrAudioTest",details)
            }
            assertTrue("Actual AudioTrack starvation recovery",player.recoveries>=3)
        } finally {player.release()}
    }

    @Test fun stoppedRealDecoderRecoversAudioWithoutRecreatingPlayer() {
        val packets=tonePackets()
        val player=AudioPlayer()
        var id=0
        fun fresh(count:Int) {
            repeat(count/2) {
                val capture=System.nanoTime()/1000
                repeat(2) {part -> player.play(AudioPacket(id++,capture+part*10_000,packets[id%packets.size]))}
                Thread.sleep(20)
            }
        }
        try {
            player.prepare()
            fresh(60)
            val before=player.acceptedFrames
            // Fault the real codec on its owning executor. Reflection stays in the
            // test APK; production exposes no decoder mutation/test interface.
            val executor=AudioPlayer::class.java.getDeclaredField("exec").apply {isAccessible=true}
                .get(player) as java.util.concurrent.ExecutorService
            executor.submit {
                val codec=AudioPlayer::class.java.getDeclaredField("codec").apply {isAccessible=true}
                    .get(player) as android.media.MediaCodec
                codec.stop()
            }.get(2,java.util.concurrent.TimeUnit.SECONDS)
            fresh(120)
            Thread.sleep(15)
            val details=player.diagnostics()
            assertTrue("The real codec fault was observed: $details",player.audioFailures>=1)
            assertTrue("Same player resumes fresh PCM: $details",player.acceptedFrames>=before+90*480)
            assertTrue("Recovered audio remains current: $details",player.lastQueuedAgeUs<=50_000)
            assertEquals("Restart clears obsolete decoder timing: $details",0,player.timelineMisses)
            assertEquals("No incomplete PCM after recovery: $details",0,player.incompleteWrites)
            android.util.Log.i("PcrAudioTest","codec fault recovery $details")
        } finally {player.release()}
    }

    @Test fun uninterruptedPlaybackStaysLiveForThreeMinutes() {
        val packets=tonePackets()
        val player=AudioPlayer()
        var id=0
        try {
            player.prepare()
            val start=System.nanoTime()
            var checkedAt=start
            var acceptedAt=player.acceptedFrames
            while(System.nanoTime()-start<180_000_000_000L) {
                val capture=System.nanoTime()/1000
                repeat(2) {part -> player.play(AudioPacket(id++,capture+part*10_000,packets[id%packets.size]))}
                Thread.sleep(20)
                val now=System.nanoTime()
                if(now-checkedAt>=1_000_000_000L) {
                    val details="elapsedSeconds=${(now-start)/1_000_000_000} ${player.diagnostics()}"
                    assertTrue("Audio continues every second: $details",player.acceptedFrames>=acceptedAt+70*480)
                    assertTrue("Capture latency stays bounded: $details",player.lastQueuedAgeUs<=50_000)
                    assertEquals("No unexplained codec/output failures: $details",0,player.audioFailures)
                    assertEquals("Every decoded buffer keeps capture timing: $details",0,player.timelineMisses)
                    assertEquals("No lost PCM tail: $details",0,player.incompleteWrites)
                    checkedAt=now
                    acceptedAt=player.acceptedFrames
                }
            }
            android.util.Log.i("PcrAudioTest","three-minute soak ${player.diagnostics()}")
        } finally {player.release()}
    }

}
