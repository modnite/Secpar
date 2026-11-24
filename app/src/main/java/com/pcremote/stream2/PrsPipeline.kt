package com.pcremote.stream2

import android.util.Base64
import android.util.Log
import android.view.Surface
import com.pcremote.agent.Msg
import com.pcremote.agent.RELAY_HOST
import com.pcremote.agent.RelayInfo
import com.pcremote.prs.AudioPacket
import com.pcremote.prs.AudioPlayer
import com.pcremote.prs.Crypt
import com.pcremote.prs.Cursor
import com.pcremote.prs.Frame
import com.pcremote.prs.Receiver
import com.pcremote.prs.ReceiverListener
import com.pcremote.prs.Stats
import com.pcremote.prs.VideoDecoder
import com.pcremote.remote.PeerSocket
import com.pcremote.remote.StunMsg
import com.pcremote.remote.TurnCreds
import kotlinx.coroutines.flow.StateFlow
import java.net.InetSocketAddress
import kotlin.concurrent.thread

/**
 * The real chain: [PeerSocket] → [Receiver] → [VideoDecoder] on `surface`, plus Opus audio.
 * Sends to every host candidate until one answers; after 2.5 s of silence, relays through TURN.
 */
class PrsPipeline(
    ready: Msg.StreamReady,
    agentHost: String,
    private val sock: PeerSocket,
    private val surface: Surface,
    relay: RelayInfo?,
    sendAgent: (Msg) -> Unit,
    private val onDecoderFailed: () -> Unit,
    private val onEnded: () -> Unit,
    private val appVsyncOffsetNs: Long = 0,
    /** Full colour is too much for this device: of 30 or more frames a second, under 80% shown, for 10 seconds. */
    private val onFullColourTooSlow: () -> Unit = {},
    /** A friend's computer has no relay key: the account's TURN credentials instead. */
    guestTurn: com.pcremote.remote.TurnServer? = null,
) : Pipeline {
    private val audio: AudioPlayer
    @Volatile private lateinit var decoder: VideoDecoder
    /** Unpacks full-colour frames onto the view; null for normal colour. */
    @Volatile private var packed: com.pcremote.prs.PackedRenderer? = null
    private val receiver: Receiver
    @Volatile private var closed = false

    init {
        val direct = agentHost.takeIf { it != RELAY_HOST }?.let { StunMsg.parseHostPort("$it:${ready.port}") }
        sock.setTargets(listOfNotNull(direct) + ready.cands.mapNotNull(StunMsg::parseHostPort))
        Log.i(TAG, "host candidates ${ready.cands}, direct $direct")
        val listener = object : ReceiverListener {
            private var reports = 0
            override fun onFrame(f: Frame) = decoder.submit(f)
            override fun onAudio(a: AudioPacket) = audio.play(a)
            override fun onBye() = onEnded()
            override fun onStats(stats: Stats) {
                if (++reports % 10 == 0) Log.i("PcrReceive", "fps=${stats.fps} kbps=${stats.kbps} loss=${stats.lossPct} rttMs=${stats.rttMs} assemblyDrops=${stats.dropped} invalid=${stats.invalidPackets} processPeakMs=${stats.processPeakMs} backlog=${stats.packetBacklog} socketDrops=${stats.socketDrops} audioMissing=${stats.audioMissing} audioReordered=${stats.audioReordered}")
            }
        }
        receiver = Receiver(sock, Crypt(Base64.decode(ready.key, Base64.DEFAULT), ready.session.toInt()), listener)
        audio = AudioPlayer(receiver::audioCaptureToLocalUs)
        decoder = newDecoder(ready)
        thread(name = "pcr-rx", priority = Thread.MAX_PRIORITY) {
            runCatching { audio.prepare() }.onFailure { Log.w(TAG, "Audio preparation failed", it) }
            if (!closed) receiver.run()
        }
        if (relay != null || guestTurn != null) thread(name = "pcr-turn-start") {
            Thread.sleep(2_500)
            if (closed || sock.confirmed()) return@thread
            val t = guestTurn ?: relay?.let { TurnCreds.fetch(it) } ?: run { Log.i(TAG, "no direct path and no TURN relay configured"); return@thread }
            val server = runCatching { InetSocketAddress(t.host, t.port) }.getOrNull()
            // UDP first; networks that block it (many offices) still allow TLS on 443.
            val relayed = server?.let { sock.useTurn(it, t.user, t.pass) }
                ?: t.tlsHost?.takeIf { !closed }?.let { host -> sock.openTls(host, t.tlsPort)?.let { sock.useTurn(it, t.user, t.pass) } }
                ?: run { Log.w(TAG, "TURN allocation failed"); return@thread }
            StunMsg.hostPort(relayed)?.let { if (!closed) { Log.i(TAG, "relaying via TURN at $it"); sendAgent(Msg.StreamCand(it)) } }
        }
    }

    private fun newDecoder(r: Msg.StreamReady): VideoDecoder {
        packed?.release()
        // Full colour: the frames are twice the picture's height, decoded into the renderer.
        val renderer = if (r.chroma == "444") com.pcremote.prs.PackedRenderer(surface, r.w, r.h) { onDecoderFailed() } else null
        packed = renderer
        // A renderer with its own input takes the decoder's output directly; otherwise frame by frame.
        val direct = renderer?.input
        return VideoDecoder(direct ?: surface, r.codec, r.w, if (renderer != null) r.h * 2 else r.h, r.fps, { onDecoderFailed() }, receiver::requestIdr, appVsyncOffsetNs,
            if (renderer != null && direct == null) renderer::frame else null)
            .also { d ->
                d.onRendered = { receiver.decodeMs = d.decodeMs }
                if (renderer != null) {
                    var slow = 0
                    var drawnAt = renderer.drawn
                    // A still screen sends few frames: only frames that arrived and were not drawn count. The
                    // decoder can keep up while the GPU merges frames, so the renderer's own count decides.
                    d.onPace = { _, arrived ->
                        val drawn = (renderer.drawn - drawnAt) / 5
                        drawnAt = renderer.drawn
                        Log.i(TAG, "full colour drawnFps=$drawn arrivedFps=$arrived drawMs=${"%.1f".format(renderer.drawMs)}")
                        slow = if (arrived >= 30 && drawn < arrived * 0.8) slow + 1 else 0
                        if (slow == 2) onFullColourTooSlow()
                    }
                }
            }
    }

    /** The host's relayed address (`stream.cand` from the host): talk relay-to-relay. */
    fun addCandidate(c: String) {
        val a = StunMsg.parseHostPort(c) ?: return
        Log.i(TAG, "host candidate $c")
        sock.addTarget(a)
        thread(name = "pcr-turn-permit") { sock.turn?.permit(listOf(a)) }
    }

    override val stats: StateFlow<Stats> get() = receiver.stats
    val route get() = sock.route
    val decoderName get() = decoder.name
    val fullColour get() = packed != null

    private var rateDecoder: VideoDecoder? = null
    private var rateAtNs = 0L
    private var decodedAt = 0
    private var drawnAt = 0
    private var rates = -1f to -1f

    /** Decoded and drawn frames per second over the last half second or more, as the desktop strip shows. */
    fun frameRates(): Pair<Float, Float> {
        val d = decoder
        val drawn = packed?.drawn ?: d.rendered
        val now = System.nanoTime()
        if (d !== rateDecoder) { rateDecoder = d; rateAtNs = now; decodedAt = d.decoded; drawnAt = drawn; return rates }
        val span = (now - rateAtNs) / 1e9f
        if (span >= 0.5f) {
            rates = (d.decoded - decodedAt).coerceAtLeast(0) / span to (drawn - drawnAt).coerceAtLeast(0) / span
            rateAtNs = now; decodedAt = d.decoded; drawnAt = drawn
        }
        return rates
    }
    fun audioDetails() = audio.details()
    fun reopenAudio() = audio.reopen()
    fun setMuted(on: Boolean) = audio.setMuted(on)
    val cursor: StateFlow<Cursor> get() = receiver.cursor

    override fun sendInput(m: Msg) = receiver.sendInput(m)

    override fun reconfigure(ready: Msg.StreamReady) {
        decoder.release() // frees the surface before the new decoder configures on the next keyframe
        decoder = newDecoder(ready)
        receiver.requestIdr()
    }

    override fun close() {
        closed = true
        decoder.release()
        packed?.release()
        audio.release()
        thread(name = "pcr-rx-stop") { receiver.stop() }
    }

    private companion object { const val TAG = "PcrPipeline" }
}
