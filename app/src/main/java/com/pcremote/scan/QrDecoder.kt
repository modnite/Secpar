package com.pcremote.scan

import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.common.GlobalHistogramBinarizer
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
        fun read(source: com.google.zxing.LuminanceSource): String? {
            for (bitmap in listOf(BinaryBitmap(HybridBinarizer(source)), BinaryBitmap(GlobalHistogramBinarizer(source)))) {
                try { return QRCodeReader().decode(bitmap, hints).text } catch (_: com.google.zxing.ReaderException) { }
                try { return QRCodeReader().decode(bitmap, hints + (DecodeHintType.PURE_BARCODE to true)).text } catch (_: com.google.zxing.ReaderException) { }
            }
            return null
        }
        read(src)?.let { return it }
        // Screen-scaled QR modules can be only 3-4 camera pixels wide. Upscaling changes
        // detector sampling without changing the payload; use a second threshold method too.
        if (maxOf(width, height) <= 1000) {
            val large = ByteArray(width * height * 4)
            for (r in 0 until height * 2) for (c in 0 until width * 2)
                large[r * width * 2 + c] = packed[(r / 2) * width + c / 2]
            read(PlanarYUVLuminanceSource(large, width * 2, height * 2, 0, 0, width * 2, height * 2, false))?.let { return it }
        }
        return read(src.invert())
    }
}
