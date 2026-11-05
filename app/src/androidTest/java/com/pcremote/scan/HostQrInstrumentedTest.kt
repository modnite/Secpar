package com.pcremote.scan

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Matrix
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.google.android.gms.tasks.Tasks
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import java.util.concurrent.TimeUnit

@RunWith(AndroidJUnit4::class)
class HostQrInstrumentedTest {
    @Test fun decodesActualHostImageAtDisplaySizeAndWithRotation() {
        val assets = InstrumentationRegistry.getInstrumentation().context.assets
        val bitmap = assets.open("host-qr.png").use { BitmapFactory.decodeStream(it) }
        val expected = assets.open("host-qr.txt").bufferedReader().use { it.readText() }
        val worker = java.util.concurrent.Executors.newSingleThreadExecutor()
        val scanner = CameraQrScanner(worker)
        try {
            val displayed = Bitmap.createScaledBitmap(bitmap, 320, 320, false)
            // A small preview may locate the code; the still capture supplies detail.
            repeat(9) {
                Tasks.await(scanner.process(InputImage.fromBitmap(displayed, 0)) { null }, 30, TimeUnit.SECONDS)
            }
            val rotatedFull = Bitmap.createBitmap(bitmap, 0, 0, bitmap.width, bitmap.height, Matrix().apply { postRotate(90f) }, false)
            // Model the detailed next frame after camera zoom/refocus, plus rotation.
            for ((photo, rotation) in listOf(bitmap to 0, rotatedFull to 270)) {
                val image = InputImage.fromBitmap(photo, rotation)
                val result = Tasks.await(scanner.process(image) {
                    val pixels = ByteArray(photo.width * photo.height) { i ->
                        val rgb = photo.getPixel(i % photo.width, i / photo.width)
                        (((rgb shr 16 and 255) * 299 + (rgb shr 8 and 255) * 587 + (rgb and 255) * 114) / 1000).toByte()
                    }
                    QrDecoder.decode(pixels, photo.width, photo.height, photo.width)
                }, 30, TimeUnit.SECONDS)
                assertEquals("Image ${image.width}x${image.height} rotation ${image.rotationDegrees}", expected, result.text)
            }
        } finally { scanner.close(); worker.shutdown() }
    }
}
