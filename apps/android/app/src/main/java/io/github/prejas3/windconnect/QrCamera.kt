package io.github.prejas3.windconnect

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.ImageFormat
import android.graphics.Matrix
import android.graphics.SurfaceTexture
import android.hardware.camera2.*
import android.media.ImageReader
import android.os.Handler
import android.os.HandlerThread
import android.view.Surface
import android.view.TextureView
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.google.zxing.*
import com.google.zxing.common.HybridBinarizer
import java.util.concurrent.atomic.AtomicBoolean

/** CCS/1 SEC:camera frames stay in RAM; only QR text reaches the strict Rust parser. No image file/network. */
@Composable
fun QrCamera(modifier: Modifier, onCode: (String) -> Unit, onError: () -> Unit) {
    val lifecycle = LocalLifecycleOwner.current
    val code by rememberUpdatedState(onCode)
    val error by rememberUpdatedState(onError)
    val context = LocalContext.current
    // CCS/1 RULE:stable owner before effects run; setting Compose state from AndroidView.factory can dispose the same worker on its first recomposition.
    val preview = remember(context) { QrCameraView(context, { code(it) }, { error() }) }
    AndroidView(modifier = modifier.clipToBounds(), factory = { preview })
    DisposableEffect(preview, lifecycle) {
        val observer = object : DefaultLifecycleObserver {
            override fun onResume(owner: LifecycleOwner) { preview.resumeCamera() }
            override fun onPause(owner: LifecycleOwner) { preview.pauseCamera() }
        }
        lifecycle.lifecycle.addObserver(observer)
        onDispose { lifecycle.lifecycle.removeObserver(observer); preview.disposeCamera() }
    }
}

internal class QrCameraView(context: Context, private val onCode: (String) -> Unit, private val onError: () -> Unit) : TextureView(context), TextureView.SurfaceTextureListener {
    private val thread = HandlerThread("WindConnect QR camera").apply { start() }
    private val worker = Handler(thread.looper)
    private val active = AtomicBoolean(false)
    private val delivered = AtomicBoolean(false)
    private var generation = 0
    private var opening = false
    private var camera: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    private var images: ImageReader? = null
    private var previewSurface: Surface? = null
    private var previewSize: android.util.Size? = null
    private var sensorOrientation = 90
    private val decoder = MultiFormatReader().apply { setHints(mapOf(DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE))) }
    private var lastFrame = 0L
    init { surfaceTextureListener = this; contentDescription = str(R.string.scan_preview) }

    fun resumeCamera() { active.set(true); if (isAvailable) worker.post { openCamera() } }
    fun pauseCamera() { active.set(false); worker.post { closeCamera() } }
    fun disposeCamera() { active.set(false); worker.post { closeCamera(); thread.quitSafely() } }
    private fun closeCamera() {
        generation++
        opening = false
        delivered.set(false)
        session?.close(); session = null
        camera?.close(); camera = null
        images?.close(); images = null
        previewSurface?.release(); previewSurface = null
    }
    private fun fail() { closeCamera(); if (active.get()) post { onError() } }

    @SuppressLint("MissingPermission")
    private fun openCamera() {
        if (!active.get() || camera != null || opening || !isAvailable) return
        opening = true
        if (context.checkSelfPermission(Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) { fail(); return }
        try {
            val manager = context.getSystemService(CameraManager::class.java)
            val id = manager.cameraIdList.firstOrNull { manager.getCameraCharacteristics(it).get(CameraCharacteristics.LENS_FACING) == CameraCharacteristics.LENS_FACING_BACK } ?: manager.cameraIdList.firstOrNull() ?: run { fail(); return }
            val characteristics = manager.getCameraCharacteristics(id)
            val sizes = characteristics.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)?.getOutputSizes(ImageFormat.YUV_420_888) ?: run { fail(); return }
            val size = sizes.filter { it.width in 320..1280 && it.height in 240..960 }.minByOrNull { kotlin.math.abs(it.width - 640) + kotlin.math.abs(it.height - 480) } ?: sizes.minBy { it.width.toLong() * it.height }
            val texture = surfaceTexture ?: return
            texture.setDefaultBufferSize(size.width, size.height)
            val surface = Surface(texture)
            previewSurface = surface
            val reader = ImageReader.newInstance(size.width, size.height, ImageFormat.YUV_420_888, 2)
            images = reader
            reader.setOnImageAvailableListener({ source ->
                val image = try { source.acquireLatestImage() } catch (_: IllegalStateException) { null }
                image?.use {
                    val now = android.os.SystemClock.elapsedRealtime()
                    if (!active.get() || delivered.get() || now - lastFrame < 200) return@use
                    lastFrame = now
                    val plane = it.planes[0]
                    val buffer = plane.buffer
                    val data = ByteArray(it.width * it.height)
                    for (y in 0 until it.height) for (x in 0 until it.width) data[y * it.width + x] = buffer.get(y * plane.rowStride + x * plane.pixelStride)
                    decodeQr(data, it.width, it.height, decoder)?.let { text ->
                        if (delivered.compareAndSet(false, true)) post { if (active.get()) onCode(text) }
                    }
                }
            }, worker)
            previewSize = size
            sensorOrientation = characteristics.get(CameraCharacteristics.SENSOR_ORIENTATION) ?: 90
            post { updateTransform() }
            val openedGeneration = ++generation
            manager.openCamera(id, object : CameraDevice.StateCallback() {
                override fun onOpened(device: CameraDevice) {
                    if (!active.get() || generation != openedGeneration) { device.close(); return }
                    opening = false
                    camera = device
                    try {
                        @Suppress("DEPRECATION")
                        device.createCaptureSession(listOf(surface, reader.surface), object : CameraCaptureSession.StateCallback() {
                            override fun onConfigured(configured: CameraCaptureSession) {
                                if (!active.get() || generation != openedGeneration) { configured.close(); return }
                                session = configured
                                try {
                                    val request = device.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW).apply {
                                        addTarget(surface); addTarget(reader.surface)
                                        val modes = characteristics.get(CameraCharacteristics.CONTROL_AF_AVAILABLE_MODES) ?: intArrayOf()
                                        if (CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE in modes) set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE)
                                    }.build()
                                    configured.setRepeatingRequest(request, null, worker)
                                } catch (_: Exception) { fail() }
                            }
                            override fun onConfigureFailed(configured: CameraCaptureSession) { configured.close(); if (generation == openedGeneration) fail() }
                        }, worker)
                    } catch (_: Exception) { fail() }
                }
                override fun onDisconnected(device: CameraDevice) { device.close(); if (generation == openedGeneration) fail() }
                override fun onError(device: CameraDevice, error: Int) { device.close(); if (generation == openedGeneration) fail() }
            }, worker)
        } catch (_: Exception) { fail() }
    }
    override fun onSurfaceTextureAvailable(surface: SurfaceTexture, width: Int, height: Int) { if (active.get()) worker.post { openCamera() } }
    private fun updateTransform() {
        val size = previewSize ?: return
        if (width == 0 || height == 0) return
        val degrees = intArrayOf(0, 90, 180, 270)[display?.rotation ?: Surface.ROTATION_0]
        // CCS/1 RULE:TextureView already applies sensor orientation. Undo stretch, rotate only display, then center-crop inside clipped preview bounds.
        val bufferWidth = (if (sensorOrientation % 180 == 0) size.width else size.height).toFloat()
        val bufferHeight = (if (sensorOrientation % 180 == 0) size.height else size.width).toFloat()
        val rotatedWidth = if (degrees % 180 == 0) bufferWidth else bufferHeight
        val rotatedHeight = if (degrees % 180 == 0) bufferHeight else bufferWidth
        val scale = maxOf(width / rotatedWidth, height / rotatedHeight)
        val matrix = Matrix()
        matrix.setScale(bufferWidth / width, bufferHeight / height)
        matrix.postTranslate(-bufferWidth / 2, -bufferHeight / 2)
        matrix.postRotate(-degrees.toFloat())
        matrix.postScale(scale, scale)
        matrix.postTranslate(width / 2f, height / 2f)
        setTransform(matrix)
    }
    override fun onSurfaceTextureSizeChanged(surface: SurfaceTexture, width: Int, height: Int) { updateTransform() }
    override fun onSurfaceTextureUpdated(surface: SurfaceTexture) = Unit
    override fun onSurfaceTextureDestroyed(surface: SurfaceTexture): Boolean { pauseCamera(); return true }
}

internal fun decodeQr(data: ByteArray, width: Int, height: Int, reader: MultiFormatReader = MultiFormatReader()): String? = try {
    val source = PlanarYUVLuminanceSource(data, width, height, 0, 0, width, height, false)
    val result = reader.decodeWithState(BinaryBitmap(HybridBinarizer(source)))
    result.text.takeIf { result.barcodeFormat == BarcodeFormat.QR_CODE && it.length <= 1024 }
} catch (_: ReaderException) { null } finally { reader.reset() }
