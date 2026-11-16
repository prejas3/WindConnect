package io.github.prejas3.windconnect.core

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.DataInputStream
import java.io.DataOutputStream
import java.security.MessageDigest

/** What the UI shows of an entry: a one-line preview or a thumbnail, never the full text or PNG. */
data class ClipboardHistoryView(
    val id: Long, val image: Boolean, val preview: String, val thumbnail: ByteArray?, val fromPc: Boolean, val pinned: Boolean,
) {
    override fun equals(other: Any?) = other is ClipboardHistoryView && other.id == id && other.pinned == pinned && other.preview == preview
    override fun hashCode() = id.hashCode() * 31 + pinned.hashCode()
}

/** An entry's content to copy back, with where it came from (its flags). */
sealed class ClipboardHistoryItem {
    abstract val fromPc: Boolean
    class Text(val text: String, override val fromPc: Boolean) : ClipboardHistoryItem()
    class Image(val png: ByteArray, val thumbnail: ByteArray, val width: Long, val height: Long, override val fromPc: Boolean) : ClipboardHistoryItem()
}

enum class PinResult { Ok, Missing, Full, TooLarge }

/**
 * CCS/1 PURPOSE:clipboard history on the phone (WN-117), the rules of the PC's (WN-37, clipboard_history.rs).
 * RULE:20 recent + up to 20 pinned (pinned on top, outside the twenty); the same content (SHA-256) moves to the top instead
 * of repeating. Unpinned images ≤ 64 MB in all (the oldest goes), pinned ones ≤ 64 MB too.
 * SEC:memory only; [encodePins] is for the encrypted pin file alone. Not thread-safe: the caller holds its lock.
 */
class ClipboardHistory {
    private class Entry(val id: Long, val item: ClipboardHistoryItem, val hash: String, val at: Long, var pinned: Boolean = false) {
        val imageBytes get() = (item as? ClipboardHistoryItem.Image)?.let { it.png.size + it.thumbnail.size } ?: 0
    }
    /** Newest first. */
    private val entries = mutableListOf<Entry>()
    private var nextId = 0L

    fun record(item: ClipboardHistoryItem, at: Long = System.currentTimeMillis()): Long {
        val hash = hash(item)
        entries.indexOfFirst { it.hash == hash }.takeIf { it >= 0 }?.let { index ->
            if (entries[index].pinned) return entries[index].id
            entries.removeAt(index)
        }
        val id = ++nextId
        entries.add(0, Entry(id, item, hash, at))
        trim(id)
        return id
    }
    private fun trim(keep: Long) {
        while (entries.count { !it.pinned } > MAX_RECENT) {
            val index = entries.indexOfLast { !it.pinned && it.id != keep }
            if (index < 0) break
            entries.removeAt(index)
        }
        while (entries.filter { !it.pinned }.sumOf { it.imageBytes.toLong() } > IMAGE_BUDGET) {
            val index = entries.indexOfLast { !it.pinned && it.id != keep && it.imageBytes > 0 }
            if (index < 0) break
            entries.removeAt(index)
        }
    }
    /** Removes it; true when it was pinned (the pin file changes). */
    fun remove(id: Long): Boolean? {
        val index = entries.indexOfFirst { it.id == id }.takeIf { it >= 0 } ?: return null
        return entries.removeAt(index).pinned
    }
    /** Removes the unpinned entries. */
    fun clear() { entries.removeAll { !it.pinned } }
    fun pin(id: Long, pinned: Boolean): PinResult {
        val entry = entries.firstOrNull { it.id == id } ?: return PinResult.Missing
        if (entry.pinned == pinned) return PinResult.Ok
        if (pinned) {
            val held = entries.filter { it.pinned }
            if (held.size >= MAX_PINNED) return PinResult.Full
            if (held.sumOf { it.imageBytes.toLong() } + entry.imageBytes > IMAGE_BUDGET) return PinResult.TooLarge
        }
        entry.pinned = pinned
        if (!pinned) trim(id)
        return PinResult.Ok
    }
    fun get(id: Long): ClipboardHistoryItem? = entries.firstOrNull { it.id == id }?.item
    val hasPins get() = entries.any { it.pinned }

    /** Pinned first, then the recent ones; newest first in each. */
    fun views(): List<ClipboardHistoryView> = (entries.filter { it.pinned } + entries.filter { !it.pinned }).map { entry ->
        when (val item = entry.item) {
            is ClipboardHistoryItem.Text -> ClipboardHistoryView(entry.id, false, preview(item.text), null, item.fromPc, entry.pinned)
            is ClipboardHistoryItem.Image -> ClipboardHistoryView(entry.id, true, "Obraz ${item.width}×${item.height}", item.thumbnail, item.fromPc, entry.pinned)
        }
    }

    /** The pinned entries, oldest first, as plain bytes for the encrypted file. */
    fun encodePins(): ByteArray {
        val out = ByteArrayOutputStream()
        DataOutputStream(out).use { data ->
            val pins = entries.filter { it.pinned }.asReversed()
            data.writeInt(pins.size)
            for (entry in pins) {
                data.writeLong(entry.at)
                data.writeBoolean(entry.item.fromPc)
                when (val item = entry.item) {
                    is ClipboardHistoryItem.Text -> { data.writeByte(KIND_TEXT); bytes(data, item.text.toByteArray(Charsets.UTF_8)) }
                    is ClipboardHistoryItem.Image -> {
                        data.writeByte(KIND_IMAGE); data.writeLong(item.width); data.writeLong(item.height)
                        bytes(data, item.thumbnail); bytes(data, item.png)
                    }
                }
            }
        }
        return out.toByteArray()
    }
    /** Adds the pins from [encodePins]; all or nothing. */
    fun loadPins(plain: ByteArray): Boolean {
        val loaded = try {
            DataInputStream(ByteArrayInputStream(plain)).use { data ->
                val count = data.readInt()
                if (count !in 0..MAX_PINNED) return false
                List(count) {
                    val at = data.readLong()
                    val fromPc = data.readBoolean()
                    val item = when (data.readByte().toInt()) {
                        KIND_TEXT -> ClipboardHistoryItem.Text(String(bytes(data, SystemClipboard.MAX_TEXT), Charsets.UTF_8), fromPc)
                        KIND_IMAGE -> {
                            val width = data.readLong(); val height = data.readLong()
                            val thumbnail = bytes(data, IMAGE_BUDGET.toInt())
                            ClipboardHistoryItem.Image(bytes(data, IMAGE_BUDGET.toInt()), thumbnail, width, height, fromPc)
                        }
                        else -> return false
                    }
                    at to item
                }.also { if (data.read() != -1) return false }
            }
        } catch (_: java.io.IOException) { return false }
        for ((at, item) in loaded) { val id = record(item, at); entries.first { it.id == id }.pinned = true }
        return true
    }

    companion object {
        const val MAX_RECENT = 20
        const val MAX_PINNED = 20
        const val IMAGE_BUDGET = 64_000_000L
        const val PREVIEW_CHARS = 120
        private const val KIND_TEXT = 1
        private const val KIND_IMAGE = 2

        private fun hash(item: ClipboardHistoryItem): String {
            val digest = MessageDigest.getInstance("SHA-256")
            digest.update((if (item is ClipboardHistoryItem.Text) KIND_TEXT else KIND_IMAGE).toByte())
            digest.update(when (item) { is ClipboardHistoryItem.Text -> item.text.toByteArray(Charsets.UTF_8); is ClipboardHistoryItem.Image -> item.png })
            return digest.digest().joinToString("") { "%02x".format(it) }
        }
        private fun bytes(data: DataOutputStream, value: ByteArray) { data.writeInt(value.size); data.write(value) }
        private fun bytes(data: DataInputStream, max: Int): ByteArray {
            val size = data.readInt()
            if (size !in 0..max) throw java.io.IOException("entry too large")
            return ByteArray(size).also { data.readFully(it) }
        }

        /** One line, ≤ 120 characters; control, bidi and zero-width characters become a space (as on the PC). */
        fun preview(text: String): String {
            val out = StringBuilder()
            var count = 0
            var index = 0
            while (index < text.length && count < PREVIEW_CHARS) {
                val code = text.codePointAt(index)
                index += Character.charCount(code)
                val hidden = Character.isISOControl(code) || code in 0x202A..0x202E || code in 0x2066..0x2069 || code in 0x200B..0x200F ||
                    code == 0xFEFF || code == 0x2028 || code == 0x2029
                out.appendCodePoint(if (hidden) ' '.code else code)
                count++
            }
            val line = out.toString().trim()
            return if (index < text.length) "$line…" else line
        }
    }
}
