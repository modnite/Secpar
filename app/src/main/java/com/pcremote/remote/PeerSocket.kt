package com.pcremote.remote

import android.util.Log
import com.pcremote.prs.Transport
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.Inet4Address
import java.net.InetSocketAddress
import java.net.NetworkInterface
import java.net.SocketException
import java.nio.ByteBuffer
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.CompletableFuture
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit

/**
 * The phone's PRS socket for one stream (docs/remote-protocol.md): learns its own candidates with
 * STUN, sends to every host candidate until an authenticated packet arrives ([confirm]), then only
 * to that address, and can fall back to a TURN relay. A reader thread demuxes STUN/TURN from PRS.
 */
class PeerSocket(private val stunServer: InetSocketAddress? = null) : Transport {
    private val sock = DatagramSocket().apply { receiveBufferSize = 4 shl 20; sendBufferSize = 1 shl 20 }
    private val prs = ArrayBlockingQueue<Triple<ByteArray, InetSocketAddress, Boolean>>(1024)
    private val discarded = java.util.concurrent.atomic.AtomicInteger()
    private val pending = ConcurrentHashMap<String, CompletableFuture<StunMsg>>()
    @Volatile private var targets: List<InetSocketAddress> = emptyList()
    @Volatile private var peer: InetSocketAddress? = null
    @Volatile private var lastFrom: InetSocketAddress? = null
    @Volatile private var lastRelayed = false
    /** The peer's packets come through our TURN relay, so ours go back that way. */
    @Volatile var viaRelay = false; private set
    /** After a path switch, packets still arriving on the old path don't move the peer back. */
    @Volatile private var holdUntilNs = 0L
    @Volatile var turn: TurnClient? = null; private set
    @Volatile private var closed = false
    /** TURN over TLS, when UDP to the relay is blocked. Its address stands for the relay in [rawSend]. */
    @Volatile private var tls: TlsLink? = null

    private val _route = kotlinx.coroutines.flow.MutableStateFlow("Connecting")
    val route: kotlinx.coroutines.flow.StateFlow<String> = _route

    val localPort get() = sock.localPort

    private val reader = Thread({ readLoop() }, "pcr-sock").apply { priority = Thread.MAX_PRIORITY; isDaemon = true; start() }

    private fun readLoop() {
        val buf = ByteArray(2048)
        val p = DatagramPacket(buf, buf.size)
        while (!closed) {
            try { p.setData(buf); sock.receive(p) } catch (_: Exception) { if (closed) return else continue }
            dispatch(buf, p.length, p.socketAddress as InetSocketAddress)
        }
    }

    /** One datagram, or one framed message from the TLS relay link. */
    private fun dispatch(buf: ByteArray, n: Int, from: InetSocketAddress) {
            val first = buf[0].toInt() and 0xFF
            when {
                first in 0x40..0x7F && from == turn?.server -> turn?.unwrap(buf, n)?.let { (data, peerAddr) -> offer(data, peerAddr, true) }
                first <= 0x03 -> StunMsg.decode(buf, n)?.let { m ->
                    val relayed = if (from == turn?.server) turn?.onIndication(m) else null
                    if (relayed != null) offer(relayed.first, relayed.second, true) else pending.remove(key(m.txid))?.complete(m)
                }
                else -> offer(buf.copyOf(n), from, false)
            }
    }

    private fun offer(b: ByteArray, from: InetSocketAddress, relayed: Boolean) {
        val item = Triple(b, from, relayed)
        if (!prs.offer(item)) { if (prs.poll() != null) discarded.incrementAndGet(); prs.offer(item) }
    }
    override fun backlog() = prs.size
    override fun droppedPackets() = discarded.get()

    private fun key(t: ByteArray) = t.joinToString("") { "%02x".format(it) }

    /** Sends a STUN/TURN request and waits for the matching response (retransmits at 0, 250, 750 ms). */
    fun transact(m: StunMsg, to: InetSocketAddress, key: ByteArray? = null, timeoutMs: Long = 1500): StunMsg? {
        val f = CompletableFuture<StunMsg>()
        pending[key(m.txid)] = f
        val bytes = m.encode(key)
        val start = System.currentTimeMillis()
        try {
            for (at in longArrayOf(0, 250, 750)) {
                val wait = start + at - System.currentTimeMillis()
                if (wait > 0) runCatching { return f.get(wait, TimeUnit.MILLISECONDS) }
                if (f.isDone) return f.get()
                rawSend(bytes, to)
            }
            return runCatching { f.get(maxOf(1, start + timeoutMs - System.currentTimeMillis()), TimeUnit.MILLISECONDS) }.getOrNull()
        } finally { pending.remove(key(m.txid)) }
    }

    /** This socket's candidates: LAN addresses plus the STUN-reflexive one. Blocking (≤ 1.5 s). */
    fun candidates(): List<String> {
        val port = sock.localPort
        val lan = runCatching {
            NetworkInterface.getNetworkInterfaces().toList().filter { it.isUp && !it.isLoopback }
                .flatMap { it.inetAddresses.toList() }.filterIsInstance<Inet4Address>().map { "${it.hostAddress}:$port" }
        }.getOrDefault(emptyList())
        val reflexive = stunServer?.let { s -> transact(StunMsg(StunMsg.BINDING), s)?.let(StunMsg::mapped)?.let(StunMsg::hostPort) }
        return (lan + listOfNotNull(reflexive)).distinct()
    }

    /** Where to send before a peer is confirmed. */
    fun setTargets(t: List<InetSocketAddress>) { targets = t.distinct() }

    fun addTarget(a: InetSocketAddress) { targets = (targets + a).distinct() }

    fun confirmed() = peer != null

    /** Opens TURN over TLS (RFC 8656 over TCP framing) and returns the address that stands for it. */
    fun openTls(host: String, port: Int): InetSocketAddress? {
        val link = runCatching { TlsLink(host, port) { b, n, from -> dispatch(b, n, from) } }.onFailure { Log.w(TAG, "TURN over TLS: ${it.message}") }.getOrNull() ?: return null
        tls = link
        return link.addr
    }

    /** Starts relaying through TURN: allocate, permit the host candidates, return the relayed address. */
    fun useTurn(server: InetSocketAddress, user: String, pass: String): InetSocketAddress? {
        val t = TurnClient(this, server, user, pass)
        val relayed = t.allocate() ?: return null
        turn = t // before permitting, so the first relayed packets are recognised
        t.permit(targets)
        return relayed
    }

    private fun rawSend(b: ByteArray, to: InetSocketAddress) {
        val link = tls
        if (link != null && to == link.addr) link.write(b) else runCatching { sock.send(DatagramPacket(b, b.size, to)) }
    }

    internal fun sendRaw(b: ByteArray, to: InetSocketAddress) = rawSend(b, to)

    override fun send(b: ByteArray) {
        val t = turn
        val p = peer
        when {
            p != null && t != null && viaRelay -> t.send(b, p)
            p != null -> rawSend(b, p)
            else -> {
                for (a in targets) rawSend(b, a)
                t?.let { tc -> for (a in targets) tc.send(b, a) }
            }
        }
    }

    override fun receive(buf: ByteArray, timeoutMs: Int): Int {
        val (b, from, relayed) = prs.poll(timeoutMs.toLong(), TimeUnit.MILLISECONDS) ?: return -1
        lastFrom = from
        lastRelayed = relayed
        b.copyInto(buf)
        return b.size
    }

    /** The last packet authenticated: talk only to where it came from (follows address changes too). */
    override fun confirm() {
        val f = lastFrom ?: return
        if (peer != null && System.nanoTime() < holdUntilNs) return
        val relayed = lastRelayed
        _route.value = if (relayed) "Relay" else "Direct"
        if (f != peer || relayed != viaRelay) { Log.i(TAG, "peer ${if (peer == null) "found" else "moved"}: $f${if (relayed) " (relay)" else ""}"); peer = f; viaRelay = relayed }
    }

    override fun otherPathReady() = peer != null && (viaRelay || turn != null)

    override fun sendOther(b: ByteArray) {
        val p = peer ?: return
        if (viaRelay) rawSend(b, p) else turn?.send(b, p)
    }

    override fun switchPath() {
        viaRelay = !viaRelay
        holdUntilNs = System.nanoTime() + 2_000_000_000L
        _route.value = if (viaRelay) "Relay" else "Direct"
        Log.i(TAG, "switched to the ${if (viaRelay) "relay" else "direct path"}: it answered faster")
    }

    override fun close() {
        closed = true
        turn?.close()
        tls?.close()
        sock.close()
    }

    companion object {
        private const val TAG = "PcrPeer"
        val CLOUDFLARE_STUN get() = runCatching { InetSocketAddress("stun.cloudflare.com", 3478) }.getOrNull()
    }
}
