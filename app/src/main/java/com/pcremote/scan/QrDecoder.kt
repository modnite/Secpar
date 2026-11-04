package com.pcremote.scan

import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader

/** Finds a QR code in a camera luminance (Y) plane. */
object QrDecoder {
    private val hints = mapOf(
        DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE),
        DecodeHintType.TRY_HARDER to true,
    )

    fun decode(y: ByteArray, width: Int, height: Int, rowStride: Int, pixelStride: Int = 1): String? {
        if (width <= 0 || height <= 0 || pixelStride <= 0 || rowStride < (width - 1) * pixelStride + 1) return null
        if ((height - 1).toLong() * rowStride + (width - 1).toLong() * pixelStride >= y.size) return null
        // Camera buffers may omit padding after the final row or interleave luminance samples.
        val packed = ByteArray(width * height)
        for (r in 0 until height) for (c in 0 until width) packed[r * width + c] = y[r * rowStride + c * pixelStride]
        val src = PlanarYUVLuminanceSource(packed, width, height, 0, 0, width, height, false)
        return try {
            QRCodeReader().decode(BinaryBitmap(HybridBinarizer(src)), hints).text
        } catch (e: Exception) {
            null // nothing found, or a blurry frame failed its checksum: try the next frame
        }
    }
}
