package io.github.prejas3.windconnect

import android.content.ContentValues
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.os.Bundle
import android.os.SystemClock
import android.provider.MediaStore
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.core.WindCore
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import java.io.ByteArrayOutputStream
import java.io.File

/**
 * CCS/1 PURPOSE:WN-58 the phone side of check-gallery-android.cjs: "Photos on the computer" on (photo access granted by the
 * script), the session held while the PC shows the photos. Commands from cache/wn58-cmd: "photo <n>" saves a new 4000×3000
 * JPEG the way a camera does (pending, bytes, published) into Pictures/WindConnectTest and acks "<n> <id> <ms>" (the id, and
 * how long the insert itself took) in cache/wn58-ack; "screenshot <n>" the same into Pictures/Screenshots; "off", "on", "done".
 * The JPEGs are encoded before any command, so the timing is the insert and nothing else.
 */
object GalleryProof {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    private fun jpeg(seed: Int): ByteArray {
        val bitmap = Bitmap.createBitmap(4000, 3000, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        canvas.drawColor(Color.rgb(40 + seed * 50 % 200, 120, 200 - seed * 30 % 150))
        val paint = Paint().apply { color = Color.WHITE; textSize = 600f }
        canvas.drawText("WN58 $seed", 300f, 1700f, paint)
        return ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.JPEG, 92, it) }.toByteArray()
    }

    private fun save(name: String, folder: String, bytes: ByteArray): Long {
        val resolver = instrumentation.targetContext.contentResolver
        val images = MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        val uri = resolver.insert(images, ContentValues().apply {
            put(MediaStore.Images.Media.DISPLAY_NAME, name)
            put(MediaStore.Images.Media.MIME_TYPE, "image/jpeg")
            put(MediaStore.Images.Media.RELATIVE_PATH, folder)
            put(MediaStore.Images.Media.IS_PENDING, 1)
        }) ?: error("insert failed")
        resolver.openOutputStream(uri)!!.use { it.write(bytes) }
        resolver.update(uri, ContentValues().apply { put(MediaStore.Images.Media.IS_PENDING, 0) }, null, null)
        return android.content.ContentUris.parseId(uri)
    }

    fun hold(core: WindCore) = runBlocking {
        val context = instrumentation.targetContext
        val command = File(context.cacheDir, "wn58-cmd").apply { delete() }
        val ack = File(context.cacheDir, "wn58-ack").apply { delete() }
        val ready = (1..4).associateWith { jpeg(it) }
        try {
            core.refreshGallery()
            check(core.galleryAccess.value) { "photo access not granted" }
            core.setGalleryAllowed(true)
            instrumentation.sendStatus(0, Bundle().apply { putString("galleryProofReady", "true") })
            withTimeout(400_000) {
                while (true) {
                    if (!command.exists()) { delay(20); continue }
                    val line = command.readText().trim()
                    command.delete()
                    val parts = line.split(' ')
                    when (parts[0]) {
                        "photo", "screenshot" -> {
                            val n = parts[1].toInt()
                            val start = SystemClock.elapsedRealtime()
                            val id = if (parts[0] == "photo") save("WN58_$n.jpg", "Pictures/WindConnectTest", ready.getValue(n))
                                else save("Screenshot_WN58_$n.jpg", "Pictures/Screenshots", ready.getValue(n))
                            val took = SystemClock.elapsedRealtime() - start
                            File(context.cacheDir, "wn58-ack.tmp").apply { writeText("$n $id $took") }.renameTo(ack)
                        }
                        "off" -> core.setGalleryAllowed(false)
                        "on" -> core.setGalleryAllowed(true)
                        "done" -> break
                    }
                }
            }
        } finally {
            command.delete()
            core.setGalleryAllowed(false)
        }
    }
}
