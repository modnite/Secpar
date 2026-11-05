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
    data class Region(val left: Int, val top: Int, val right: Int, val bottom: Int) {
        /** ML Kit reports bounds in the rotated image; CameraX Y rows are unrotated. */
        fun unrotate(width: Int, height: Int, degrees: Int): Region = when (degrees) {
            90 -> Region(top, height - right, bottom, height - left)
            180 -> Region(width - right, height - bottom, width - left, height - top)
            270 -> Region(width - bottom, left, width - top, right)
            else -> this
        }
    }
    private val hints = mapOf(
        DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE),
        DecodeHintType.TRY_HARDER to true,
    )

    fun decode(y: ByteArray, width: Int, height: Int, rowStride: Int, pixelStride: Int = 1, regions: List<Region> = emptyList()): String? {
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
        // A monitor bezel or dark UI can defeat PURE_BARCODE over the whole frame.
        // Use the native detector's potential boxes even when it cannot decode text.
        for (region in regions.take(8)) {
            val margin = (maxOf(region.right - region.left, region.bottom - region.top) / 10).coerceAtLeast(4)
            val left = (region.left - margin).coerceIn(0, width - 1)
            val top = (region.top - margin).coerceIn(0, height - 1)
            val right = (region.right + margin).coerceIn(left + 1, width)
            val bottom = (region.bottom + margin).coerceIn(top + 1, height)
            read(src.crop(left, top, right - left, bottom - top))?.let { return it }
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
