package com.pcremote.scan

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.Camera
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageCapture
import androidx.camera.core.ImageCaptureException
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.FlashlightOff
import androidx.compose.material.icons.rounded.FlashlightOn
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.pcremote.R
import com.pcremote.ui.components.PrimaryButton
import com.pcremote.ui.theme.LocalMidnight
import com.pcremote.ui.theme.MidnightTheme
import com.pcremote.ui.theme.rememberHaptics
import com.google.mlkit.vision.common.InputImage
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/** Starts the scanner and hands back the first QR text it reads. */
object QrScan {
    @Volatile private var pending: ((String) -> Unit)? = null

    fun start(activity: Activity, onResult: (String) -> Unit) {
        pending = onResult
        activity.startActivity(Intent(activity, ScanActivity::class.java))
    }

    fun cancel() { pending = null }

    internal fun deliver(text: String) {
        pending?.invoke(text)
        pending = null
    }
}

/** CameraX with the bundled on-device ML Kit QR detector; no model download is needed. */
class ScanActivity : ComponentActivity() {
    private val analyzer = Executors.newSingleThreadExecutor()
    private val done = AtomicBoolean(false)
    @Volatile private var boundCamera: Camera? = null
    private var detailCapture: ImageCapture? = null
    private val capturingDetail = AtomicBoolean(false)
    private var lastDetailAt = 0L
    private val scanner = CameraQrScanner(analyzer) { ratio ->
        boundCamera?.let { cam ->
            val state = cam.cameraInfo.zoomState.value
            val target = ratio.coerceIn(state?.minZoomRatio ?: 1f, state?.maxZoomRatio ?: 1f)
            if (target > (state?.zoomRatio ?: 1f)) cam.cameraControl.setZoomRatio(target)
            true
        } ?: false
    }
    private var scanStatus by mutableStateOf("Starting camera…")
    private var frames = 0
    private val openedAt = android.os.SystemClock.elapsedRealtime()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent { MidnightTheme(dark = true) { Scanner() } }
    }

    override fun onDestroy() {
        super.onDestroy()
        analyzer.shutdown()
        boundCamera = null
        detailCapture = null
        scanner.close()
    }

    @Composable
    private fun Scanner() {
        val c = LocalMidnight.current
        val haptics = rememberHaptics()
        val owner = LocalLifecycleOwner.current
        LaunchedEffect(Unit) {
            kotlinx.coroutines.delay(8_000)
            if (frames == 0 && !done.get()) scanStatus = "No camera frames processed. Reopen the scanner or paste a pairing link."
        }
        var granted by remember {
            mutableStateOf(ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED)
        }
        val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted = it }
        LaunchedEffect(Unit) { if (!granted) ask.launch(Manifest.permission.CAMERA) }
        var torch by remember { mutableStateOf(false) }
        var camera by remember { mutableStateOf<Camera?>(null) }

        Box(Modifier.fillMaxSize().background(Color.Black)) {
            if (granted) {
                AndroidView(factory = { ctx ->
                    val view = PreviewView(ctx).apply { scaleType = PreviewView.ScaleType.FILL_CENTER }
                    val providerFuture = ProcessCameraProvider.getInstance(ctx)
                    providerFuture.addListener({
                      try {
                        val provider = providerFuture.get()
                        val preview = Preview.Builder().build().also { it.surfaceProvider = view.surfaceProvider }
                        // Analysis previews can locate a dense code without retaining
                        // enough pixels to decode it. Take a real full-resolution still.
                        val detail = ImageCapture.Builder()
                            .setCaptureMode(ImageCapture.CAPTURE_MODE_MAXIMIZE_QUALITY)
                            .build()
                        val analysis = ImageAnalysis.Builder()
                            .setTargetResolution(android.util.Size(1920, 1080))
                            .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                            .build()
                        analysis.setAnalyzer(analyzer) { proxy ->
                            val media = proxy.image
                            if (media == null || done.get()) {
                                proxy.close()
                            } else {
                              try {
                                // Keep ImageProxy open until asynchronous scanning completes.
                                val image = InputImage.fromMediaImage(media, proxy.imageInfo.rotationDegrees)
                                scanner.process(image) { candidates ->
                                    val plane = proxy.planes[0]
                                    val buffer = plane.buffer.duplicate()
                                    val bytes = ByteArray(buffer.remaining()).also { buffer.get(it) }
                                    val regions = candidates.mapNotNull { it.boundingBox }.map {
                                        QrDecoder.Region(it.left, it.top, it.right, it.bottom)
                                            .unrotate(proxy.width, proxy.height, proxy.imageInfo.rotationDegrees)
                                    }
                                    QrDecoder.decode(bytes, proxy.width, proxy.height, plane.rowStride, plane.pixelStride, regions)
                                }
                                    .addOnSuccessListener { result ->
                                        frames++
                                        val text = result.text
                                        scanStatus = when {
                                            text != null -> "Code read"
                                            capturingDetail.get() -> "Reading code… Hold steady."
                                            result.potential -> "QR detected. Reading details…"
                                            android.os.SystemClock.elapsedRealtime() - openedAt > 8_000 ->
                                                "No QR detected. Tap to focus, increase zoom, or paste a pairing link."
                                            else -> "Scanning…"
                                        }
                                        if (text != null && done.compareAndSet(false, true)) {
                                            haptics.click()
                                            QrScan.deliver(text)
                                            finish()
                                        } else if (result.potential && !done.get()) {
                                            readDetail(view) { decoded ->
                                                if (done.compareAndSet(false, true)) {
                                                    haptics.click()
                                                    QrScan.deliver(decoded)
                                                    finish()
                                                }
                                            }
                                        }
                                    }
                                    .addOnFailureListener { error ->
                                        android.util.Log.e("SecparScan", "Camera decoder failed", error)
                                        scanStatus = "Scanner error. You can paste a pairing link instead."
                                    }
                                    .addOnCompleteListener { proxy.close() }
                              } catch (error: Exception) {
                                proxy.close()
                                android.util.Log.e("SecparScan", "Unsupported camera image", error)
                                runOnUiThread { scanStatus = "Camera image could not be read. Paste a pairing link instead." }
                              }
                            }
                        }
                        provider.unbindAll()
                        camera = try {
                            provider.bindToLifecycle(owner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis, detail)
                                .also { detailCapture = detail }
                        } catch (_: IllegalArgumentException) {
                            // Older cameras may not support all three streams together.
                            provider.unbindAll()
                            provider.bindToLifecycle(owner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis)
                        }
                        boundCamera = camera
                        view.setOnTouchListener { _, event ->
                            if (event.action == android.view.MotionEvent.ACTION_UP) {
                                val point = view.meteringPointFactory.createPoint(event.x, event.y)
                                camera?.cameraControl?.startFocusAndMetering(
                                    androidx.camera.core.FocusMeteringAction.Builder(point)
                                        .setAutoCancelDuration(3, java.util.concurrent.TimeUnit.SECONDS).build())
                                view.performClick()
                            }
                            true
                        }
                        view.post {
                            val center = view.meteringPointFactory.createPoint(view.width / 2f, view.height / 2f)
                            camera?.cameraControl?.startFocusAndMetering(androidx.camera.core.FocusMeteringAction.Builder(center).build())
                        }
                      } catch (error: Exception) {
                        android.util.Log.e("SecparScan", "Could not bind camera analysis", error)
                        scanStatus = "Camera could not start scanning. Reopen it or paste a pairing link."
                      }
                    }, ContextCompat.getMainExecutor(ctx))
                    view
                }, modifier = Modifier.fillMaxSize())
                Viewfinder()
            } else {
                Column(Modifier.align(Alignment.Center).padding(32.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    Text(stringResource(R.string.pcr_scan_need_camera), color = c.text, textAlign = TextAlign.Center,
                        style = MaterialTheme.typography.bodyLarge)
                    Spacer(Modifier.height(20.dp))
                    PrimaryButton(stringResource(R.string.pcr_scan_allow), { ask.launch(Manifest.permission.CAMERA) })
                }
            }
            Row(Modifier.fillMaxWidth().safeDrawingPadding().padding(8.dp), verticalAlignment = Alignment.CenterVertically) {
                IconButton(::finish) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, null, tint = Color.White) }
                Spacer(Modifier.weight(1f))
                if (camera?.cameraInfo?.hasFlashUnit() == true) {
                    IconButton({ torch = !torch; camera?.cameraControl?.enableTorch(torch) }) {
                        Icon(if (torch) Icons.Rounded.FlashlightOn else Icons.Rounded.FlashlightOff, null, tint = Color.White)
                    }
                }
            }
            Column(Modifier.align(Alignment.BottomCenter).safeDrawingPadding().padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    for (ratio in listOf(1f, 2f, 3f)) {
                        androidx.compose.material3.TextButton(onClick = {
                            camera?.let { cam ->
                                val state = cam.cameraInfo.zoomState.value
                                cam.cameraControl.setZoomRatio(ratio.coerceIn(state?.minZoomRatio ?: 1f, state?.maxZoomRatio ?: 1f))
                            }
                        }) { Text("${ratio.toInt()}×", color = Color.White) }
                    }
                    androidx.compose.material3.TextButton(onClick = {
                        QrScan.cancel()
                        startActivity(Intent(this@ScanActivity, com.pcremote.MainActivity::class.java).putExtra("secpar_paste_pairing", true))
                        finish()
                    }) { Text("Paste link", color = Color.White) }
                }
            Text(scanStatus, color = Color.White, textAlign = TextAlign.Center,
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.fillMaxWidth())
            }
        }
    }

    private fun readDetail(view: PreviewView, deliver: (String) -> Unit) {
        val capture = detailCapture ?: return
        val now = android.os.SystemClock.elapsedRealtime()
        if (now - lastDetailAt < 4000 || !capturingDetail.compareAndSet(false, true)) return
        lastDetailAt = now
        scanStatus = "Reading code… Hold steady."
        val center = view.meteringPointFactory.createPoint(view.width / 2f, view.height / 2f)
        val focus = boundCamera?.cameraControl?.startFocusAndMetering(
            androidx.camera.core.FocusMeteringAction.Builder(center)
                .setAutoCancelDuration(2, java.util.concurrent.TimeUnit.SECONDS).build())
        val take = Runnable {
            if (isDestroyed || done.get()) { capturingDetail.set(false); return@Runnable }
            capture.takePicture(analyzer, object : ImageCapture.OnImageCapturedCallback() {
                override fun onCaptureSuccess(image: ImageProxy) {
                    try {
                        // CameraX stills are JPEG. Decode the original pixels rather
                        // than interpreting compressed bytes as a camera Y plane.
                        val buffer = image.planes[0].buffer.duplicate()
                        val bytes = ByteArray(buffer.remaining()).also { buffer.get(it) }
                        val bitmap = android.graphics.BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
                            ?: throw IllegalStateException("Invalid camera photo")
                        android.util.Log.d("SecparScan", "Detailed camera frame ${bitmap.width}x${bitmap.height}")
                        scanner.process(InputImage.fromBitmap(bitmap, image.imageInfo.rotationDegrees)) { candidates ->
                            val pixels = IntArray(bitmap.width * bitmap.height)
                            bitmap.getPixels(pixels, 0, bitmap.width, 0, 0, bitmap.width, bitmap.height)
                            val luminance = ByteArray(pixels.size) { i ->
                                val rgb = pixels[i]
                                (((rgb shr 16 and 255) * 299 + (rgb shr 8 and 255) * 587 + (rgb and 255) * 114) / 1000).toByte()
                            }
                            val regions = candidates.mapNotNull { it.boundingBox }.map {
                                QrDecoder.Region(it.left, it.top, it.right, it.bottom)
                                    .unrotate(bitmap.width, bitmap.height, image.imageInfo.rotationDegrees)
                            }
                            QrDecoder.decode(luminance, bitmap.width, bitmap.height, bitmap.width, regions = regions)
                        }.addOnSuccessListener { result ->
                            if (result.text != null) android.util.Log.d("SecparScan", "Detailed QR decoded")
                            if (!isDestroyed && result.text != null) deliver(result.text)
                        }.addOnFailureListener {
                            if (!isDestroyed) scanStatus = "Move closer to the code and hold steady."
                        }.addOnCompleteListener {
                            bitmap.recycle()
                            image.close()
                            capturingDetail.set(false)
                        }
                    } catch (error: Exception) {
                        image.close()
                        capturingDetail.set(false)
                        android.util.Log.w("SecparScan", "Detailed scan failed", error)
                    }
                }
                override fun onError(exception: ImageCaptureException) {
                    capturingDetail.set(false)
                    runOnUiThread { if (!isDestroyed) scanStatus = "Move closer to the code and hold steady." }
                }
            })
        }
        if (focus != null) focus.addListener(take, ContextCompat.getMainExecutor(this))
        else take.run()
    }

    /** Dimmed surround with a clear rounded window and an accent outline. */
    @Composable
    private fun Viewfinder() {
        val accent = LocalMidnight.current.accent
        Canvas(Modifier.fillMaxSize().graphicsLayer(compositingStrategy = CompositingStrategy.Offscreen)) {
            val side = size.minDimension * 0.68f
            val tl = Offset((size.width - side) / 2, (size.height - side) / 2.4f)
            drawRect(Color.Black.copy(alpha = 0.55f))
            drawRoundRect(Color.Transparent, tl, Size(side, side), CornerRadius(36.dp.toPx()), blendMode = BlendMode.Clear)
            drawRoundRect(accent, tl, Size(side, side), CornerRadius(36.dp.toPx()), style = Stroke(4.dp.toPx()))
        }
    }
}
