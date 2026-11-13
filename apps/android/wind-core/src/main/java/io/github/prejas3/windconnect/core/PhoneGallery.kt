package io.github.prejas3.windconnect.core

import android.Manifest
import android.content.ContentResolver
import android.content.ContentUris
import android.content.Context
import android.content.pm.PackageManager
import android.database.ContentObserver
import android.graphics.Bitmap
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.provider.MediaStore
import android.util.Size
import io.github.prejas3.windconnect.bindings.GalleryItem
import io.github.prejas3.windconnect.bindings.GalleryItems
import io.github.prejas3.windconnect.bindings.GalleryOriginal
import io.github.prejas3.windconnect.bindings.GallerySource
import io.github.prejas3.windconnect.bindings.GalleryThumb
import java.io.ByteArrayOutputStream

/**
 * CCS/1 M:WN-58 the phone's newest photos and screenshots for the PC's panel (PROTOCOL.md §9.13), read from MediaStore on
 * the core's blocking threads. RULE:photo access checked on every call (revoking it answers "denied" at once); pending and
 * trashed items never leave (MediaStore hides them by default); an original goes as a detached descriptor the core closes.
 * Nothing is cached on disk and no name is logged.
 */
class PhoneGallery(context: Context) : GallerySource {
    private val app = context.applicationContext
    private val resolver: ContentResolver = app.contentResolver

    /** READ_MEDIA_IMAGES (Android 13+) or READ_EXTERNAL_STORAGE (10–12). */
    fun canRead(): Boolean = app.checkSelfPermission(PERMISSION) == PackageManager.PERMISSION_GRANTED

    private inline fun <T> guarded(fail: (UByte) -> T, block: () -> T): T = when {
        !canRead() -> fail(DENIED)
        else -> try { block() } catch (_: SecurityException) { fail(DENIED) } catch (_: Exception) { fail(FAILED) }
    }

    override fun recent(limit: UInt): GalleryItems = guarded({ GalleryItems(it, emptyList()) }) {
        val items = ArrayList<GalleryItem>(limit.toInt())
        resolver.query(IMAGES, arrayOf(MediaStore.Images.Media._ID, MediaStore.Images.Media.DATE_ADDED,
            MediaStore.Images.Media.DISPLAY_NAME, MediaStore.Images.Media.MIME_TYPE, MediaStore.Images.Media.WIDTH,
            MediaStore.Images.Media.HEIGHT, MediaStore.Images.Media.SIZE, MediaStore.Images.Media.RELATIVE_PATH),
            null, null, "${MediaStore.Images.Media.DATE_ADDED} DESC, ${MediaStore.Images.Media._ID} DESC",
        )?.use { c ->
            while (c.moveToNext() && items.size < limit.toInt()) {
                val id = c.getLong(0)
                val size = c.getLong(6)
                if (id < 0 || size <= 0) continue
                items += GalleryItem(id.toULong(), (c.getLong(1).coerceAtLeast(0) * 1000).toULong(),
                    c.getString(2)?.takeIf { it.isNotBlank() } ?: "IMG_$id", c.getString(3) ?: "image/jpeg",
                    c.getInt(4).coerceAtLeast(0).toUInt(), c.getInt(5).coerceAtLeast(0).toUInt(), size.toULong(),
                    GalleryRules.isScreenshot(c.getString(7)))
            }
        } ?: return@guarded GalleryItems(FAILED, emptyList())
        GalleryItems(OK, items)
    }

    /** MediaStore's own cached thumbnail, as a JPEG within the protocol's 64 KiB. */
    override fun thumb(id: ULong, size: UInt): GalleryThumb = guarded({ GalleryThumb(it, byteArrayOf()) }) {
        val px = size.toInt()
        val bitmap = try {
            resolver.loadThumbnail(uri(id), Size(px, px), null)
        } catch (_: java.io.FileNotFoundException) { return@guarded GalleryThumb(NOT_FOUND, byteArrayOf()) }
        val fitted = if (maxOf(bitmap.width, bitmap.height) > px) {
            val scale = px.toFloat() / maxOf(bitmap.width, bitmap.height)
            Bitmap.createScaledBitmap(bitmap, (bitmap.width * scale).toInt().coerceAtLeast(1), (bitmap.height * scale).toInt().coerceAtLeast(1), true)
        } else bitmap
        var quality = 80
        var jpeg: ByteArray
        do {
            jpeg = ByteArrayOutputStream().also { fitted.compress(Bitmap.CompressFormat.JPEG, quality, it) }.toByteArray()
            quality -= 15
        } while (jpeg.size > MAX_THUMB && quality > 20)
        if (jpeg.size > MAX_THUMB) GalleryThumb(FAILED, byteArrayOf()) else GalleryThumb(OK, jpeg)
    }

    override fun original(id: ULong): GalleryOriginal = guarded({ GalleryOriginal(it, "", "", 0u, -1) }) {
        val uri = uri(id)
        val meta = resolver.query(uri, arrayOf(MediaStore.Images.Media.DISPLAY_NAME, MediaStore.Images.Media.MIME_TYPE), null, null, null)
            ?.use { c -> if (c.moveToFirst()) (c.getString(0) ?: "") to (c.getString(1) ?: "image/jpeg") else null }
            ?: return@guarded GalleryOriginal(NOT_FOUND, "", "", 0u, -1)
        val pfd = try { resolver.openFileDescriptor(uri, "r") } catch (_: java.io.FileNotFoundException) { null }
            ?: return@guarded GalleryOriginal(NOT_FOUND, "", "", 0u, -1)
        val size = pfd.statSize
        if (size <= 0) { pfd.close(); return@guarded GalleryOriginal(FAILED, "", "", 0u, -1) }
        // From here the core owns the descriptor and closes it.
        GalleryOriginal(OK, meta.first, meta.second, size.toULong(), pfd.detachFd())
    }

    private fun uri(id: ULong): Uri = ContentUris.withAppendedId(IMAGES, id.toLong())

    // CCS/1 FLOW:MediaStore change -> debounce in the core (CHANGE_DEBOUNCE) -> Changed to every PC watch -> the PC asks again.
    private var observer: ContentObserver? = null
    private var thread: HandlerThread? = null

    /** Watches the phone's images while [on]; [changed] runs on a background thread. */
    @Synchronized fun observe(on: Boolean, changed: () -> Unit) {
        if (on == (observer != null)) return
        if (!on) {
            observer?.let(resolver::unregisterContentObserver)
            observer = null
            thread?.quitSafely(); thread = null
            return
        }
        val looper = HandlerThread("wind-gallery-observer").also { it.start(); thread = it }.looper
        val o = object : ContentObserver(Handler(looper)) {
            override fun onChange(selfChange: Boolean) = changed()
            // Every volume notifies under content://media/<volume>/images/…; audio and video changes are not ours.
            override fun onChange(selfChange: Boolean, uri: Uri?) {
                if (uri == null || uri.pathSegments.getOrNull(1) == "images") changed()
            }
        }
        try {
            resolver.registerContentObserver(MediaStore.AUTHORITY_URI, true, o)
            observer = o
        } catch (_: SecurityException) {
            thread?.quitSafely(); thread = null
        }
    }

    companion object {
        val PERMISSION: String = if (Build.VERSION.SDK_INT >= 33) Manifest.permission.READ_MEDIA_IMAGES
            else Manifest.permission.READ_EXTERNAL_STORAGE
        private val IMAGES: Uri = MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL)
        private const val OK: UByte = 0u
        private const val DENIED: UByte = 1u
        private const val NOT_FOUND: UByte = 2u
        private const val FAILED: UByte = 4u
        private const val MAX_THUMB = 64 * 1024
    }
}

/** WN-58: plain rules of [PhoneGallery], testable without Android. */
object GalleryRules {
    /** Screenshots live in a "Screenshots" folder (DCIM/ or Pictures/, any case). */
    fun isScreenshot(relativePath: String?): Boolean =
        relativePath?.split('/')?.any { it.equals("Screenshots", ignoreCase = true) } == true
}
