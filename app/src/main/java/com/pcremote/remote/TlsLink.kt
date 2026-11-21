package com.pcremote.remote

import android.util.Log
import java.io.DataInputStream
import java.io.OutputStream
import java.net.InetSocketAddress
import javax.net.ssl.SSLSocket
import javax.net.ssl.SSLSocketFactory

/**
 * A TURN server over TLS (RFC 8656 section 3.1: STUN messages and ChannelData framed by their
 * own lengths, ChannelData padded to four bytes). Each framed message goes to `deliver` as if
 * it were a datagram from [addr]. The relay still talks UDP to the PC.
 */
class TlsLink(host: String, port: Int, private val deliver: (ByteArray, Int, InetSocketAddress) -> Unit) {
    val addr: InetSocketAddress = InetSocketAddress(host, port)
    private val socket: SSLSocket = (SSLSocketFactory.getDefault().createSocket() as SSLSocket).apply {
        connect(addr, 5_000)
        tcpNoDelay = true
        soTimeout = 0
        // The default factory checks the certificate against the host name.
        sslParameters = sslParameters.apply { endpointIdentificationAlgorithm = "HTTPS" }
        startHandshake()
    }
    private val out: OutputStream = socket.outputStream
    @Volatile private var closed = false

    private val reader = Thread({ readLoop() }, "pcr-turn-tls").apply { isDaemon = true; priority = Thread.MAX_PRIORITY; start() }

    private fun readLoop() {
        val input = DataInputStream(socket.inputStream.buffered(1 shl 16))
        val buf = ByteArray(4 + 0xFFFF + 20)
        try {
            while (!closed) {
                input.readFully(buf, 0, 4)
                val n = frameLength(buf)
                input.readFully(buf, 4, n - 4)
                deliver(buf, n, addr)
            }
        } catch (e: Exception) {
            if (!closed) Log.w("PcrTurn", "TURN over TLS closed: ${e.message}")
        }
    }

    @Synchronized
    fun write(b: ByteArray) {
        if (closed) return
        runCatching { out.write(b); out.flush() }
    }

    fun close() {
        closed = true
        runCatching { socket.close() }
    }

    companion object {
        /** The whole frame's length from its first four bytes. */
        fun frameLength(head: ByteArray): Int {
            val len = ((head[2].toInt() and 0xFF) shl 8) or (head[3].toInt() and 0xFF)
            return if ((head[0].toInt() and 0xC0) == 0x40) 4 + (len + 3) / 4 * 4 else 20 + len
        }
    }
}
