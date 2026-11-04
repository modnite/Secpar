package com.pcremote.scan

import com.google.android.gms.tasks.Task
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.barcode.ZoomSuggestionOptions
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage
import java.util.concurrent.Executor

/** Native camera detection plus a second decoder for dense screen-displayed QR codes. */
class CameraQrScanner(worker: Executor, zoom: (Float) -> Boolean = { false }) : AutoCloseable {
    private val worker = worker
    private val requestZoom = zoom
    private var misses = 0
    private val scanner = BarcodeScanning.getClient(BarcodeScannerOptions.Builder()
        .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
        .enableAllPotentialBarcodes()
        .setZoomSuggestionOptions(ZoomSuggestionOptions.Builder { zoom(it) }.setMaxSupportedZoomRatio(3f).build())
        .build())

    data class Result(val text: String?, val potential: Boolean)

    fun process(image: InputImage, fallback: () -> String?): Task<Result> = scanner.process(image).continueWith(worker) { task ->
        val codes = if (task.isSuccessful) task.result else emptyList()
        val native = codes.firstNotNullOfOrNull { it.rawValue?.takeIf(String::isNotBlank) }
        val text = native ?: fallback()
        if (text == null) {
            misses++
            // Even when the detector cannot find a bounding box, retry with more real
            // camera detail. Interpolation cannot recover lost modules in a small code.
            when (misses) {
                3 -> requestZoom(1.5f)
                6 -> requestZoom(2f)
                9 -> requestZoom(3f)
            }
        } else misses = 0
        if (!task.isSuccessful && text == null) throw task.exception ?: IllegalStateException("QR scanner failed")
        Result(text, codes.isNotEmpty())
    }

    override fun close() = scanner.close()
}
