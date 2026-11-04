package com.pcremote.scan

import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class QrDecoderTest {
    private val url = "pcremote://pair?n=SADIKTD007&h=192.168.0.126&p=47999&fp=" + "ab".repeat(32) + "&t=M9tGaEcnoA4efDGpUVnMag"

    /** Renders a QR into an 8-bit luminance frame like a camera Y plane, with a quiet border. */
    private fun frame(text: String, scale: Int = 6, border: Int = 40): Triple<ByteArray, Int, Int> {
        val m = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, 0, 0)
        val w = m.width * scale + border * 2; val h = m.height * scale + border * 2
        val y = ByteArray(w * h) { 0xF0.toByte() }
        for (r in 0 until m.height) for (c in 0 until m.width) if (m[c, r]) {
            for (dy in 0 until scale) for (dx in 0 until scale) y[(border + r * scale + dy) * w + border + c * scale + dx] = 0x10
        }
        return Triple(y, w, h)
    }

    @Test fun `decodes a pairing QR from a camera-like frame`() {
        val (y, w, h) = frame(url)
        assertEquals(url, QrDecoder.decode(y, w, h, rowStride = w))
    }

    @Test fun `handles row padding in the camera buffer`() {
        val (y, w, h) = frame(url)
        val stride = w + 64
        val padded = ByteArray(stride * h).also { for (r in 0 until h) System.arraycopy(y, r * w, it, r * stride, w) }
        assertEquals(url, QrDecoder.decode(padded, w, h, rowStride = stride))
    }

    @Test fun `blank frame decodes to null`() {
        assertNull(QrDecoder.decode(ByteArray(320 * 240) { 0x80.toByte() }, 320, 240, 320))
    }

    @Test fun `decodes dense remote Secpar code and parses its signed relay URL`() {
        val relay = "wss://pcr-rendezvous.example.workers.dev/v1/phone?pc=12345678-1234-1234-1234-123456789012&dev=pair_abcdefghijklmnopqrstuv&ts=1700000000&sig=" + "ab".repeat(32)
        val link = url.replace("pcremote://", "secpar://").replace("47999", "48999") +
            "&pc=12345678-1234-1234-1234-123456789012&r=" + java.net.URLEncoder.encode(relay, "UTF-8")
        val (y, w, h) = frame(link, scale = 3, border = 16)
        val decoded = QrDecoder.decode(y, w, h, w)
        assertEquals(link, decoded)
        val parsed = com.pcremote.agent.PairUri.parse(decoded!!)!!
        assertEquals(relay, parsed.relayUrl)
        assertEquals(48999, parsed.port)
    }

    @Test fun `handles pixel stride and omitted final row padding`() {
        val (y, w, h) = frame(url)
        val stride = w * 2 + 64
        val buffer = ByteArray((h - 1) * stride + (w - 1) * 2 + 1)
        for (r in 0 until h) for (c in 0 until w) buffer[r * stride + c * 2] = y[r * w + c]
        assertEquals(url, QrDecoder.decode(buffer, w, h, stride, 2))
        assertNull(QrDecoder.decode(buffer.copyOf(10), w, h, stride, 2))
    }
}
