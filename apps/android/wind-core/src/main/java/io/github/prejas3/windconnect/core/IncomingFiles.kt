package io.github.prejas3.windconnect.core

import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.MediaStore
import android.util.AtomicFile
import android.webkit.MimeTypeMap
import io.github.prejas3.windconnect.bindings.FileCommitResult
import io.github.prejas3.windconnect.bindings.FileCommitStatus
import io.github.prejas3.windconnect.bindings.FileDestination
import io.github.prejas3.windconnect.bindings.IncomingControl
import io.github.prejas3.windconnect.bindings.IncomingFile
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.io.FileInputStream
import java.security.MessageDigest
import java.text.Normalizer
import java.util.Locale

data class ReceivedFile(val key: String, val name: String, val folder: String, val size: Long, val uri: String)

/** CCS/1 SEC:only app-created Downloads rows and verified files below the private inbox enter this journal. */
class IncomingFiles(private val context: Context, private val transactionPhase: (String) -> Unit = {}) : FileDestination {
    private val resolver = context.contentResolver
    private val collection = MediaStore.Downloads.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
    private val privateRoot = File(context.noBackupFilesDir, "file-engine/inbox").canonicalFile
    private val journal = AtomicFile(File(context.noBackupFilesDir, "received-files.json"))
    private val records = linkedMapOf<String, JSONObject>()
    private val mutableReceipts = MutableStateFlow<List<ReceivedFile>>(emptyList())
    val receipts = mutableReceipts.asStateFlow()
    private val mutableError = MutableStateFlow("")
    val error = mutableError.asStateFlow()
    private val lock = Any()

    init { synchronized(lock) { load(); recover(); publishReceipts() } }

    private fun load() {
        if (!journal.baseFile.exists()) return
        val bytes = journal.openRead().use { input ->
            val output = java.io.ByteArrayOutputStream(); val buffer = ByteArray(8192)
            while (true) { val n = input.read(buffer); if (n < 0) break; require(output.size() + n <= 4 * 1024 * 1024); output.write(buffer, 0, n) }
            output.toByteArray()
        }
        require(bytes.size <= 4 * 1024 * 1024) { "Dziennik odbioru jest za duży" }
        val array = JSONArray(String(bytes, Charsets.UTF_8))
        require(array.length() <= 512)
        for (index in 0 until array.length()) {
            val record = array.getJSONObject(index)
            val key = record.getString("key")
            require(key.matches(Regex("[0-9a-f]{64}:[0-9a-f]{32}")))
            require(record.getString("phase") in setOf("pending", "ready", "published"))
            records[key] = record
        }
    }
    private fun save() {
        val bytes = JSONArray(records.values.toList()).toString().toByteArray(Charsets.UTF_8)
        require(bytes.size <= 4 * 1024 * 1024)
        val output = journal.startWrite()
        try { output.write(bytes); journal.finishWrite(output) }
        catch (failure: Throwable) { journal.failWrite(output); throw failure }
    }
    private fun uri(record: JSONObject): Uri {
        val value = Uri.parse(record.getString("uri"))
        require(value.scheme == "content" && value.authority == MediaStore.AUTHORITY)
        val parts = value.pathSegments
        require(parts.size == 3 && parts[0] == MediaStore.VOLUME_EXTERNAL_PRIMARY && parts[1] == "downloads" && parts[2].toLongOrNull() != null)
        return value
    }
    private fun readOwned(value: Uri): JSONObject? {
        resolver.query(value, arrayOf(MediaStore.MediaColumns.OWNER_PACKAGE_NAME, MediaStore.MediaColumns.DISPLAY_NAME, MediaStore.MediaColumns.RELATIVE_PATH, MediaStore.MediaColumns.IS_PENDING, MediaStore.MediaColumns.SIZE), null, null, null)?.use { cursor ->
            if (!cursor.moveToFirst()) return null
            if (cursor.getString(0) != context.packageName || cursor.getString(2)?.startsWith("Download/WindConnect/") != true) return null
            return JSONObject().put("name", cursor.getString(1)).put("folder", cursor.getString(2)).put("pending", cursor.getInt(3)).put("size", cursor.getLong(4))
        }
        return null
    }
    private fun deletePending(record: JSONObject) {
        if (!record.has("uri")) return
        val value = uri(record)
        if (readOwned(value)?.optInt("pending") == 1) resolver.delete(value, null, null)
    }
    private fun cleanPrivate(record: JSONObject) {
        if (!record.has("source")) return
        val file = File(record.getString("source"))
        if (!file.exists()) return
        require(file.canonicalFile.toPath().startsWith(privateRoot.toPath()) && file.isFile && !java.nio.file.Files.isSymbolicLink(file.toPath()))
        check(file.delete())
    }
    @Suppress("DEPRECATION")
    private fun recover() {
        val remove = mutableListOf<String>()
        for ((key, record) in records) {
            if (record.getString("phase") == "published") { cleanPrivate(record); continue }
            val actual = if (record.has("uri")) readOwned(uri(record)) else null
            if (actual != null && actual.getInt("pending") == 0) {
                require(record.getString("phase") == "ready" && record.has("sha256"))
                record.put("phase", "published").put("name", actual.getString("name")).put("folder", actual.getString("folder"))
                cleanPrivate(record)
            } else { deletePending(record); cleanPrivate(record); remove.add(key) }
        }
        remove.forEach(records::remove)
        // Insert-before-journal crashes can leave only app-owned, still-pending transaction names here.
        resolver.query(MediaStore.setIncludePending(collection), arrayOf(MediaStore.MediaColumns._ID, MediaStore.MediaColumns.OWNER_PACKAGE_NAME),
            "${MediaStore.MediaColumns.RELATIVE_PATH}=? AND ${MediaStore.MediaColumns.IS_PENDING}=1 AND ${MediaStore.MediaColumns.DISPLAY_NAME} LIKE ?",
            arrayOf("Download/WindConnect/_incoming/", "wc-%"), null)?.use { cursor ->
            while (cursor.moveToNext()) if (cursor.getString(1) == context.packageName) {
                val value = android.content.ContentUris.withAppendedId(collection, cursor.getLong(0))
                if (records.values.none { it.optString("uri") == value.toString() }) resolver.delete(value, null, null)
            }
        }
        save()
    }
    private fun publishReceipts() {
        mutableReceipts.value = records.values.filter { it.getString("phase") == "published" }.takeLast(50).reversed().map {
            ReceivedFile(it.getString("key"), it.getString("name"), it.getString("folder"), it.getLong("size"), it.getString("uri"))
        }
    }
    override fun commit(file: IncomingFile, control: IncomingControl): FileCommitResult = synchronized(lock) {
        val key = "${file.peer}:${file.id}"
        var record: JSONObject? = null
        try {
            require(key.matches(Regex("[0-9a-f]{64}:[0-9a-f]{32}")))
            val size = file.size.toLong(); require(size in 0..2L * 1024 * 1024 * 1024)
            val source = File(file.source)
            require(source.isFile && source.canonicalFile.toPath().startsWith(privateRoot.toPath()) && !java.nio.file.Files.isSymbolicLink(source.toPath()) && source.length() == size)
            val parts = receivePath(file.name)
            var previous = records[key]
            if (previous != null && previous.getString("phase") != "published") {
                val actual = if (previous.has("uri")) readOwned(uri(previous)) else null
                if (actual?.optInt("pending") == 0 && previous.has("sha256")) {
                    previous.put("phase", "published").put("name", actual.getString("name")).put("folder", actual.getString("folder")); save(); publishReceipts()
                } else { deletePending(previous); records.remove(key); save(); previous = null }
            }
            if (previous != null) {
                require(previous.getString("wire") == file.name && previous.getLong("size") == size)
                val digest = FileInputStream(source).use { digest(it, control) }
                require(previous.getString("phase") == "published" && previous.getString("sha256") == digest)
                check(readOwned(uri(previous))?.optInt("pending") == 0) { "Odebrany wcześniej plik został przeniesiony lub usunięty" }
                return@synchronized FileCommitResult(FileCommitStatus.PUBLISHED, null)
            }
            check(records.size < 512) { "Historia odbioru jest pełna. Wyczyść historię w WindConnect." }
            check(!control.isCancelled())
            val pendingName = "wc-" + MessageDigest.getInstance("SHA-256").digest(key.toByteArray()).hex() + ".pending"
            record = JSONObject().put("key", key).put("phase", "pending").put("wire", file.name).put("size", size).put("source", source.canonicalPath)
            records[key] = record; save()
            val value = resolver.insert(collection, ContentValues().apply {
                put(MediaStore.MediaColumns.DISPLAY_NAME, pendingName)
                put(MediaStore.MediaColumns.MIME_TYPE, "application/octet-stream")
                put(MediaStore.MediaColumns.RELATIVE_PATH, "Download/WindConnect/_incoming/")
                put(MediaStore.MediaColumns.IS_PENDING, 1)
            }) ?: error("Nie można utworzyć pliku w Pobranych")
            transactionPhase("inserted")
            record.put("uri", value.toString()); save()
            transactionPhase("pending")
            val hash = MessageDigest.getInstance("SHA-256")
            var copied = 0L
            resolver.openFileDescriptor(value, "w")?.use { descriptor ->
                FileInputStream(source).use { input ->
                    ParcelFileDescriptor.AutoCloseOutputStream(descriptor).use { output ->
                        val buffer = ByteArray(64 * 1024)
                        while (true) {
                            check(!control.isCancelled()) { "Anulowano odbiór" }
                            val length = input.read(buffer); if (length < 0) break
                            copied += length; require(copied <= size)
                            output.write(buffer, 0, length); hash.update(buffer, 0, length)
                            transactionPhase("copying")
                        }
                        output.flush(); output.fd.sync()
                    }
                }
            } ?: error("Nie można zapisać pliku w Pobranych")
            require(copied == size); check(!control.isCancelled())
            record.put("sha256", hash.digest().hex()).put("phase", "ready"); save()
            transactionPhase("ready")
            val name = parts.last()
            val folder = "Download/WindConnect/" + parts.dropLast(1).joinToString("/", postfix = if (parts.size > 1) "/" else "")
            val mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(name.substringAfterLast('.', "").lowercase(Locale.ROOT)) ?: "application/octet-stream"
            check(resolver.update(value, ContentValues().apply {
                put(MediaStore.MediaColumns.DISPLAY_NAME, name); put(MediaStore.MediaColumns.RELATIVE_PATH, folder)
                put(MediaStore.MediaColumns.MIME_TYPE, mime); put(MediaStore.MediaColumns.IS_PENDING, 0)
            }, null, null) == 1)
            transactionPhase("visible")
            val actual = readOwned(value) ?: error("Opublikowany plik nie jest dostępny")
            require(actual.getInt("pending") == 0 && actual.getLong("size") == size)
            record.put("phase", "published").put("name", actual.getString("name")).put("folder", actual.getString("folder")); save(); publishReceipts()
            transactionPhase("published")
            mutableError.value = ""
            FileCommitResult(FileCommitStatus.PUBLISHED, null)
        } catch (failure: Exception) {
            // Never remove a visible, already-published file after a late failure/cancel.
            try { record?.let { deletePending(it) } } catch (_: Exception) { }
            if (record != null) {
                val visible = try { record.has("uri") && readOwned(uri(record))?.optInt("pending") == 0 } catch (_: Exception) { true }
                if (!visible) { records.remove(key); try { save() } catch (_: Exception) { } }
            }
            mutableError.value = failure.message ?: "Nie udało się opublikować pliku w Pobranych"
            val noSpace = generateSequence<Throwable>(failure) { it.cause }.any { it is android.system.ErrnoException && it.errno == android.system.OsConstants.ENOSPC || it.message?.contains("ENOSPC") == true }
            FileCommitResult(if (control.isCancelled()) FileCommitStatus.CANCELLED else if (noSpace) FileCommitStatus.NO_SPACE else FileCommitStatus.FAILED, mutableError.value)
        }
    }
    fun receiptUri(key: String): Uri? = synchronized(lock) {
        val record = records[key]?.takeIf { it.optString("phase") == "published" } ?: return@synchronized null
        try { uri(record).takeIf { readOwned(it)?.optInt("pending") == 0 } } catch (_: Exception) { null }
    }
    fun clearHistory() = synchronized(lock) {
        // User explicitly forgets receipts; this never deletes public files.
        records.entries.removeAll { it.value.optString("phase") == "published" }; save(); publishReceipts()
    }
    private fun digest(input: java.io.InputStream, control: IncomingControl): String {
        val hash = MessageDigest.getInstance("SHA-256"); val bytes = ByteArray(64 * 1024)
        while (true) { check(!control.isCancelled()); val n = input.read(bytes); if (n < 0) break; hash.update(bytes, 0, n) }
        return hash.digest().hex()
    }
}

private fun ByteArray.hex() = joinToString("") { "%02x".format(it) }
internal fun receivePath(wire: String): List<String> {
    require(wire.toByteArray().size <= 4096)
    val parts = wire.split('/').filter { it.isNotEmpty() && it != "." && it != ".." }.map { segment ->
        val safe = Normalizer.normalize(segment, Normalizer.Form.NFC).filterNot { it.isISOControl() || it in '\u202a'..'\u202e' || it in '\u2066'..'\u2069' || it == '\u200e' || it == '\u200f' }.map { if (it in "\\/:*?\"<>|") '_' else it }.joinToString("").trim().trim('.')
        require(safe.isNotEmpty() && safe.toByteArray().size <= 240); safe
    }
    require(parts.isNotEmpty() && parts.size <= 16)
    return parts
}
