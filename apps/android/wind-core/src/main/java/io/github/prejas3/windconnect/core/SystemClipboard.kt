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
import io.github.prejas3.windconnect.bindings.ClipboardImageInterface
import io.github.prejas3.windconnect.bindings.ClipboardSink
import java.io.ByteArrayOutputStream
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

data class ClipboardUi(val enabled: Boolean = true, val status: String = "", val busy: Boolean = false, val delivered: Long = 0, val received: Long = 0, val imageLimitMb: Int = SystemClipboard.DEFAULT_IMAGE_LIMIT_MB)

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
    private val pinStore: ClipboardPinStore? = ClipboardPinStore(context),
    private val publish: suspend (String, Boolean, Boolean) -> String,
) : ClipboardSink {
    private val context = context.applicationContext
    private val manager = context.getSystemService(ClipboardManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private val mutable = MutableStateFlow(preferences.getBoolean("clipboard_enabled", true).let { enabled ->
        ClipboardUi(enabled, syncText(enabled),
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
    // WN-117: what went between the devices; guarded by itself, pins written one at a time.
    private val history = ClipboardHistory()
    private val historyViews = MutableStateFlow<List<ClipboardHistoryView>>(emptyList())
    val historyUi = historyViews.asStateFlow()
    @OptIn(ExperimentalCoroutinesApi::class) private val pinWriter = Dispatchers.IO.limitedParallelism(1)
    init {
        if (pinStore != null) scope.launch(pinWriter) {
            val plain = pinStore.read() ?: return@launch
            try { if (!synchronized(history) { history.loadPins(plain) }) pinStore.hold() } finally { plain.fill(0) }
            publishHistory()
        }
    }

    fun setEnabled(value: Boolean) {
        enabledChanged(value)
        preferences.edit().putBoolean("clipboard_enabled", value).apply()
        mutable.update { it.copy(enabled = value, status = syncText(value)) }
        // CCS/1 RULE:sync off keeps nothing new and drops the unpinned entries; pins stay.
        if (!value) { synchronized(history) { history.clear() }; publishHistory() }
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
            if (result == "applied") { mutable.update { it.copy(delivered = it.delivered + 1) }; record(ClipboardHistoryItem.Text(text, fromPc = false)) }
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
                is ImageRead.Ready -> publishImage(image.image, manual) { mutable.update { it.copy(status = context.getString(R.string.clip_image_sending)) } }
                is ImageRead.TooLarge -> "too-large" to null
                ImageRead.Unreadable -> "unreadable" to null
            }
            if (result.first == "applied") {
                mutable.update { it.copy(delivered = it.delivered + 1) }
                (image as ImageRead.Ready).image.let { record(ClipboardHistoryItem.Image(it.png, it.thumbnail, it.width, it.height, fromPc = false)) }
            }
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
            mutable.update { it.copy(status = context.getString(R.string.clip_received), received = it.received + 1) }
            record(ClipboardHistoryItem.Text(text, fromPc = true))
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
            val clip = ClipData(ClipDescription(context.getString(R.string.clip_image_from_pc), arrayOf(ClipImageProvider.PNG)), ClipData.Item(ClipImageProvider.uri(context, image)))
            manager.setPrimaryClip(clip)
            mutable.update { it.copy(status = context.getString(R.string.clip_image_received), received = it.received + 1) }
            recordWhenComplete(image)
            true
        } catch (_: Exception) { ClipImageProvider.publish(null); false }
    }
    /** WN-117: into the history while sync is on; never a clip marked sensitive (those are neither read nor sent). */
    private fun record(item: ClipboardHistoryItem) {
        if (closed || !mutable.value.enabled) return
        synchronized(history) { history.record(item) }
        publishHistory()
    }
    /** A received image goes into the history once its whole PNG is here; replaced or broken off, it doesn't. */
    private fun recordWhenComplete(image: ClipboardImageInterface) {
        val size = image.size().toLong()
        if (size <= 0 || size > ClipboardHistory.IMAGE_BUDGET) return
        val thumbnail = image.thumbnail()
        scope.launch(Dispatchers.IO) {
            val png = try {
                val out = ByteArrayOutputStream(size.toInt())
                while (out.size() < size) {
                    val part = image.read(out.size().toULong(), 65_536u)
                    if (part.isEmpty()) break
                    out.write(part)
                }
                out.toByteArray().takeIf { it.size.toLong() == size }
            } catch (_: Exception) { null } ?: return@launch
            record(ClipboardHistoryItem.Image(png, thumbnail, image.width().toLong(), image.height().toLong(), fromPc = true))
        }
    }
    private fun publishHistory() { historyViews.value = synchronized(history) { history.views { w, h -> context.getString(R.string.clip_image_size, w, h) } } }
    private fun savePins() {
        val store = pinStore ?: return
        scope.launch(pinWriter) {
            if (store.held) {
                // A file not loaded at start: try again and merge, or leave it as it is (WN-139).
                val saved = store.read()
                if (saved != null) {
                    try { if (!synchronized(history) { history.loadPins(saved) }) store.hold() } finally { saved.fill(0) }
                    publishHistory()
                }
                if (store.held) return@launch
            }
            val plain = synchronized(history) { if (history.hasPins) history.encodePins() else null }
            store.write(plain)
        }
    }
    /**
     * Copies an entry back with the flags it had: from the PC as at receipt (text IS_SENSITIVE; an image through our provider,
     * so neither is sent back: no echo); from the phone as plain text, which sync sends to the PC like any new copy.
     */
    fun historyCopy(id: Long): Boolean {
        val item = synchronized(history) { history.get(id) } ?: return false
        return try {
            when (item) {
                is ClipboardHistoryItem.Text -> manager.setPrimaryClip(ClipData.newPlainText("WindConnect", item.text).apply {
                    if (item.fromPc) description.extras = PersistableBundle().apply { putBoolean(SENSITIVE, true) }
                })
                is ClipboardHistoryItem.Image -> {
                    val stored = StoredClipImage(item)
                    ClipImageProvider.publish(stored)
                    manager.setPrimaryClip(ClipData(ClipDescription(context.getString(R.string.clip_image_from_history), arrayOf(ClipImageProvider.PNG)), ClipData.Item(ClipImageProvider.uri(context, stored))))
                }
            }
            mutable.update { it.copy(status = context.getString(R.string.clip_copied_from_history)) }
            true
        } catch (_: Exception) { false }
    }
    fun historyPin(id: Long, pinned: Boolean): PinResult {
        val result = synchronized(history) { history.pin(id, pinned) }
        if (result == PinResult.Ok) { savePins(); publishHistory() }
        else mutable.update { it.copy(status = when (result) {
            PinResult.Full -> context.getString(R.string.clip_pin_full, ClipboardHistory.MAX_PINNED)
            PinResult.TooLarge -> context.getString(R.string.clip_pin_too_large)
            else -> context.getString(R.string.clip_pin_gone)
        }) }
        return result
    }
    fun historyRemove(id: Long) {
        val wasPinned = synchronized(history) { history.remove(id) } ?: return
        if (wasPinned) savePins()
        publishHistory()
    }
    /** Removes the unpinned entries. */
    fun historyClear() { synchronized(history) { history.clear() }; publishHistory() }
    private fun report(result: String) { mutable.update { it.copy(status = statusText(result)) } }
    private fun syncText(enabled: Boolean) = context.getString(if (enabled) R.string.clip_sync_auto else R.string.clip_sync_off)
    /** Megabytes with at most one decimal, in the phone's number format (WN-63). */
    private fun mb(bytes: Long): String {
        val tenths = (bytes + 99_999) / 100_000
        return java.text.NumberFormat.getNumberInstance().apply { maximumFractionDigits = 1 }.format(tenths / 10.0)
    }
    private fun imageStatusText(value: String, peerLimit: Long?, limit: Long) = when (value) {
        "applied" -> context.getString(R.string.clip_image_sent)
        "too-large" -> context.getString(R.string.clip_image_too_large, mb(limit))
        "peer-limit" -> context.getString(R.string.clip_image_peer_limit, mb(peerLimit ?: 0))
        "unsupported" -> context.getString(R.string.clip_image_unsupported)
        "replaced" -> context.getString(R.string.clip_image_replaced)
        "unreadable" -> context.getString(R.string.clip_image_unreadable)
        "failed" -> context.getString(R.string.clip_image_failed)
        else -> statusText(value)
    }
    private fun statusText(value: String) = context.getString(when (value) {
        "applied" -> R.string.clip_applied
        "sensitive" -> R.string.clip_sensitive
        "unchanged" -> R.string.clip_unchanged
        "disabled" -> R.string.clip_sync_auto_off
        "refused" -> R.string.clip_refused
        "invalid" -> R.string.clip_invalid
        "empty" -> R.string.clip_empty
        "busy" -> R.string.clip_busy
        "failed" -> R.string.clip_failed
        else -> R.string.clip_unavailable
    })
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
    }
}

/** An image from the history for the provider: complete, served from memory. */
internal class StoredClipImage(private val image: ClipboardHistoryItem.Image) : ClipboardImageInterface {
    private val id = java.util.UUID.randomUUID().toString().replace("-", "")
    override fun id() = id
    override fun width() = image.width.toULong()
    override fun height() = image.height.toULong()
    override fun size() = image.png.size.toULong()
    override fun isComplete() = true
    override fun thumbnail() = image.thumbnail
    override fun read(offset: ULong, max: UInt): ByteArray {
        val start = minOf(offset, size()).toInt()
        return image.png.copyOfRange(start, minOf(image.png.size.toLong(), start.toLong() + max.toLong()).toInt())
    }
}
