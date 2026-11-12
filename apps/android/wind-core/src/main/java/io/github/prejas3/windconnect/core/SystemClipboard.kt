package io.github.prejas3.windconnect.core

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.SharedPreferences
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.os.PersistableBundle
import io.github.prejas3.windconnect.bindings.ClipboardImage
import io.github.prejas3.windconnect.bindings.ClipboardSink
import java.io.ByteArrayOutputStream
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

data class ClipboardUi(val enabled: Boolean = true, val status: String = "Synchronizacja automatyczna", val busy: Boolean = false, val delivered: Long = 0, val received: Long = 0, val imageLimitMb: Int = SystemClipboard.DEFAULT_IMAGE_LIMIT_MB)

/** An image to send: its PNG, a thumbnail PNG and the pixel size (WN-36). */
internal class OutgoingImage(val png: ByteArray, val thumbnail: ByteArray, val width: Long, val height: Long)

/**
 * CCS/1 SEC:plain text, and images (WN-36) only from an image content URI read while focused (the clip's grant), never other
 * URIs/Intents; no clipboard payload in UI/logs/storage.
 */
internal class SystemClipboard(
    context: Context, private val preferences: SharedPreferences, private val scope: CoroutineScope,
    private val enabledChanged: (Boolean) -> Unit,
    private val connected: () -> Boolean,
    private val imageLimitChanged: (Long) -> Unit = {},
    private val publishImage: suspend (OutgoingImage, Boolean, () -> Unit) -> Pair<String, Long?> = { _, _, _ -> "unavailable" to null },
    private val publish: suspend (String, Boolean, Boolean) -> String,
) : ClipboardSink {
    private val context = context.applicationContext
    private val manager = context.getSystemService(ClipboardManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private val mutable = MutableStateFlow(preferences.getBoolean("clipboard_enabled", true).let { enabled ->
        ClipboardUi(enabled, if (enabled) "Synchronizacja automatyczna" else "Synchronizacja wyłączona",
            imageLimitMb = preferences.getInt(IMAGE_LIMIT, DEFAULT_IMAGE_LIMIT_MB).takeIf { it in IMAGE_LIMITS_MB } ?: DEFAULT_IMAGE_LIMIT_MB)
    })
    val ui = mutable.asStateFlow()
    private var focused = false
    private var listening = false
    @Volatile private var closed = false
    private var sending: Job? = null
    private var dirty = false
    private val freshness = ClipboardFreshness()
    private val listener = ClipboardManager.OnPrimaryClipChangedListener { automatic() }

    fun setEnabled(value: Boolean) {
        enabledChanged(value)
        preferences.edit().putBoolean("clipboard_enabled", value).apply()
        mutable.update { it.copy(enabled = value, status = if (value) "Synchronizacja automatyczna" else "Synchronizacja wyłączona") }
        main.post { updateListener() }
    }
    /** The image limit in MB (5, 16 or 50): sent and taken images are at most this big. */
    fun setImageLimit(mb: Int) {
        if (mb !in IMAGE_LIMITS_MB) return
        preferences.edit().putInt(IMAGE_LIMIT, mb).apply()
        imageLimitChanged(mb * 1_000_000L)
        mutable.update { it.copy(imageLimitMb = mb) }
    }
    fun applyImageLimit() = imageLimitChanged(mutable.value.imageLimitMb * 1_000_000L)
    fun focus(value: Boolean) { main.post { val changed = focused != value; focused = value; updateListener(); if (value && changed) automatic() } }
    fun connectionChanged() { main.post { updateListener() } }
    fun sessionStarted() { freshness.sessionStarted(System.currentTimeMillis()) }
    private fun updateListener() {
        val wanted = !closed && focused && mutable.value.enabled && connected()
        if (wanted && !listening) manager.addPrimaryClipChangedListener(listener)
        if (!wanted && listening) manager.removePrimaryClipChangedListener(listener)
        listening = wanted
    }
    private fun automatic() {
        if (closed || !focused || !mutable.value.enabled || !connected()) return
        dirty = true
        if (sending?.isActive == true) return
        sending = scope.launch(Dispatchers.Main.immediate) {
            do { dirty = false; send(false) } while (dirty && focused && !closed && mutable.value.enabled)
        }
    }
    suspend fun send(manual: Boolean = true): String = withContext(Dispatchers.Main.immediate) {
        if (closed) return@withContext "unavailable"
        if (!connected()) { if (manual) report("unavailable"); return@withContext "unavailable" }
        val clip = try { manager.primaryClip } catch (_: SecurityException) { null }
        if (!manual && clip != null && !freshness.allows(clip.description.timestamp)) return@withContext "unchanged"
        if (clip != null && clip.itemCount == 1 && clip.getItemAt(0).text == null && clip.getItemAt(0).uri != null &&
            clip.description.hasMimeType("image/*")) {
            if (clip.description.extras?.getBoolean(SENSITIVE, false) == true) {
                if (manual) report("sensitive")
                return@withContext "sensitive"
            }
            return@withContext sendImage(clip.getItemAt(0).uri, manual)
        }
        if (clip == null || clip.itemCount != 1 || clip.getItemAt(0).text == null) {
            if (manual) report("empty"); return@withContext "empty"
        }
        if (clip.description.extras?.getBoolean(SENSITIVE, false) == true) {
            if (manual) report("sensitive")
            return@withContext "sensitive"
        }
        val value = clip.getItemAt(0).text
        if (value.length > MAX_TEXT) { if (manual) report("invalid"); return@withContext "invalid" }
        val text = value.toString()
        if (text.contains('\u0000') || text.toByteArray(Charsets.UTF_8).size > MAX_TEXT) { if (manual) report("invalid"); return@withContext "invalid" }
        mutable.update { it.copy(busy = true) }
        try {
            val result = publish(text, false, manual)
            if (result == "applied") mutable.update { it.copy(delivered = it.delivered + 1) }
            if (result != "unchanged" || manual) report(result)
            result
        } finally { mutable.update { it.copy(busy = false) } }
    }
    /** Phone → PC (WN-36): read while focused (the clip grants us the URI), PNG as is, other types re-encoded. */
    private suspend fun sendImage(uri: Uri, manual: Boolean): String {
        // CCS/1 RULE:our own received image (its URI) is never sent back: no echo.
        if (ClipImageProvider.isOurs(context, uri)) return "unchanged"
        val limit = mutable.value.imageLimitMb * 1_000_000L
        mutable.update { it.copy(busy = true) }
        try {
            val image = withContext(Dispatchers.IO) { readImage(uri, limit) }
            val result = when (image) {
                is ImageRead.Ready -> publishImage(image.image, manual) { mutable.update { it.copy(status = "Wysyłanie obrazu…") } }
                is ImageRead.TooLarge -> "too-large" to null
                ImageRead.Unreadable -> "unreadable" to null
            }
            if (result.first == "applied") mutable.update { it.copy(delivered = it.delivered + 1) }
            if (result.first != "unchanged" || manual) mutable.update { it.copy(status = imageStatusText(result.first, result.second, limit)) }
            return result.first
        } finally { mutable.update { it.copy(busy = false) } }
    }
    private sealed class ImageRead {
        class Ready(val image: OutgoingImage) : ImageRead()
        object TooLarge : ImageRead()
        object Unreadable : ImageRead()
    }
    private fun readImage(uri: Uri, limit: Long): ImageRead = try { readImageOrFail(uri, limit) } catch (_: SecurityException) { ImageRead.Unreadable } catch (_: java.io.IOException) { ImageRead.Unreadable } catch (_: OutOfMemoryError) { ImageRead.TooLarge }
    private fun readImageOrFail(uri: Uri, limit: Long): ImageRead {
        val resolver = context.contentResolver
        val type = resolver.getType(uri)
        val bytes = resolver.openInputStream(uri)?.use { input ->
            val out = ByteArrayOutputStream()
            val buffer = ByteArray(64 * 1024)
            var total = 0L
            while (true) {
                val n = input.read(buffer)
                if (n < 0) break
                total += n
                // A source over the limit may still fit once re-encoded; allow 4× for JPEG/WebP sources.
                if (total > limit * 4 || total > MAX_IMAGE) return ImageRead.TooLarge
                out.write(buffer, 0, n)
            }
            out.toByteArray()
        } ?: return ImageRead.Unreadable
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return ImageRead.Unreadable
        if (bounds.outWidth > MAX_SIDE || bounds.outHeight > MAX_SIDE) return ImageRead.TooLarge
        val png = if (type == ClipImageProvider.PNG || bounds.outMimeType == ClipImageProvider.PNG) bytes else {
            val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size) ?: return ImageRead.Unreadable
            try { ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray() } finally { bitmap.recycle() }
        }
        if (png.size > limit) return ImageRead.TooLarge
        val thumbnail = thumbnail(bytes, bounds.outWidth, bounds.outHeight) ?: return ImageRead.Unreadable
        return ImageRead.Ready(OutgoingImage(png, thumbnail, bounds.outWidth.toLong(), bounds.outHeight.toLong()))
    }

    override fun applyText(text: String): Boolean {
        if (closed || !mutable.value.enabled) return false
        // CCS/1 RULE:Binder clipboard write is synchronous; no UI-thread wait/re-entry into native clipboard state.
        return try {
            val clip = ClipData.newPlainText("WindConnect", text)
            clip.description.extras = PersistableBundle().apply { putBoolean(SENSITIVE, true) }
            manager.setPrimaryClip(clip)
            mutable.update { it.copy(status = "Schowek odebrany z komputera", received = it.received + 1) }
            true
        } catch (_: Exception) { false }
    }
    /**
     * PC → phone (WN-36): on the clipboard at once as a content URI (pasteable); the provider streams the PNG as it arrives
     * and gives previews the thumbnail. Not marked sensitive, so the system preview shows it (review point 1: images from
     * the user's own PC; a private image is never sent by the PC).
     */
    override fun applyImage(image: ClipboardImage): Boolean {
        if (closed || !mutable.value.enabled) return false
        return try {
            ClipImageProvider.publish(image)
            val clip = ClipData(ClipDescription("Obraz z komputera", arrayOf(ClipImageProvider.PNG)), ClipData.Item(ClipImageProvider.uri(context, image)))
            manager.setPrimaryClip(clip)
            mutable.update { it.copy(status = "Obraz odebrany z komputera", received = it.received + 1) }
            true
        } catch (_: Exception) { ClipImageProvider.publish(null); false }
    }
    private fun report(result: String) { mutable.update { it.copy(status = statusText(result)) } }
    fun close() { closed = true; main.post { sending?.cancel(); updateListener() } }
    companion object {
        const val SENSITIVE = "android.content.extra.IS_SENSITIVE"
        const val MAX_TEXT = 256 * 1024
        const val IMAGE_LIMIT = "clipboard_image_limit_mb"
        val IMAGE_LIMITS_MB = listOf(5, 16, 50)
        const val DEFAULT_IMAGE_LIMIT_MB = 16
        private const val MAX_IMAGE = 64L * 1024 * 1024
        private const val MAX_SIDE = 16_384
        private const val MAX_THUMBNAIL = 128 * 1024
        private const val THUMBNAIL_SIDE = 512

        /** A PNG thumbnail, long side ≤ 512 px and ≤ 128 KiB (smaller sides until it fits). */
        fun thumbnail(bytes: ByteArray, width: Int, height: Int): ByteArray? {
            var side = THUMBNAIL_SIDE
            val sample = BitmapFactory.Options().apply { inSampleSize = Integer.highestOneBit((maxOf(width, height) / THUMBNAIL_SIDE).coerceAtLeast(1)) }
            val decoded = BitmapFactory.decodeByteArray(bytes, 0, bytes.size, sample) ?: return null
            try {
                while (true) {
                    val scale = minOf(1f, side.toFloat() / maxOf(decoded.width, decoded.height))
                    val scaled = Bitmap.createScaledBitmap(decoded, (decoded.width * scale).toInt().coerceAtLeast(1), (decoded.height * scale).toInt().coerceAtLeast(1), true)
                    val png = ByteArrayOutputStream().also { scaled.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
                    if (scaled !== decoded) scaled.recycle()
                    if (png.size <= MAX_THUMBNAIL || side <= 32) return png
                    side /= 2
                }
            } finally { decoded.recycle() }
        }
        private fun mb(bytes: Long): String {
            val tenths = (bytes + 99_999) / 100_000
            return if (tenths % 10 == 0L) "${tenths / 10}" else "${tenths / 10},${tenths % 10}"
        }
        fun imageStatusText(value: String, peerLimit: Long?, limit: Long) = when (value) {
            "applied" -> "Obraz wysłany na komputer"
            "too-large" -> "Obraz przekracza limit ${mb(limit)} MB z ustawień"
            "peer-limit" -> "Komputer przyjmuje obrazy do ${mb(peerLimit ?: 0)} MB"
            "unsupported" -> "Komputer nie obsługuje obrazów — zaktualizuj aplikację"
            "replaced" -> "Nowszy schowek zastąpił obraz"
            "unreadable" -> "Nie udało się odczytać obrazu ze schowka"
            "failed" -> "Nie udało się przekazać obrazu na komputer"
            else -> statusText(value)
        }
        fun statusText(value: String) = when (value) {
            "applied" -> "Schowek dostarczony do komputera"
            "sensitive" -> "Treść oznaczona jako wrażliwa — nie wysłano"
            "unchanged" -> "Schowek jest już zsynchronizowany"
            "disabled" -> "Synchronizacja automatyczna wyłączona"
            "refused" -> "Komputer odrzucił schowek lub nowszą zmianę już zastosowano"
            "invalid" -> "Obsługiwany jest tekst do 256 KiB, bez znaku NUL"
            "empty" -> "Brak tekstu w schowku lub brak dostępu do niego"
            "busy" -> "Schowek jest zajęty — spróbuj ponownie"
            "failed" -> "Nie udało się zapisać schowka na komputerze"
            else -> "Schowek niedostępny — sprawdź połączenie z komputerem"
        }
    }
}
