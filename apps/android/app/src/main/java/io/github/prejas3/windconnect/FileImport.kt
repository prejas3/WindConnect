package io.github.prejas3.windconnect

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import kotlinx.coroutines.*
import java.io.File
import java.io.FileOutputStream
import java.text.Normalizer
import java.util.Locale

internal object FileImport {
    const val MAX_FILES = 100
    const val MAX_FILE_BYTES = 2L * 1024 * 1024 * 1024
    const val MAX_BATCH_BYTES = 4L * 1024 * 1024 * 1024
    const val RESERVE = 64L * 1024 * 1024

    @Suppress("DEPRECATION")
    fun uris(intent: Intent): List<Uri> {
        val streams: List<Uri> = when (intent.action) {
            Intent.ACTION_SEND -> listOfNotNull(if (android.os.Build.VERSION.SDK_INT >= 33)
                intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java) else intent.getParcelableExtra(Intent.EXTRA_STREAM))
            Intent.ACTION_SEND_MULTIPLE -> (if (android.os.Build.VERSION.SDK_INT >= 33)
                intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java) else intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM)) ?: emptyList()
            null -> return emptyList()
            else -> error(str(R.string.import_unsupported))
        }
        val clips = intent.clipData?.let { clip ->
            require(clip.itemCount <= MAX_FILES) { str(R.string.import_max_files) }
            (0 until clip.itemCount).map { requireNotNull(clip.getItemAt(it).uri) { str(R.string.import_not_text) } }
        } ?: emptyList()
        return checkedUris(streams + clips).also { require(it.isNotEmpty()) { str(R.string.send_none) } }
    }

    fun checkedUris(uris: List<Uri>): List<Uri> {
        require(uris.size <= MAX_FILES * 2) { str(R.string.import_too_many) }
        val unique = uris.distinct()
        require(unique.size <= MAX_FILES) { str(R.string.import_max_files) }
        val lengths = unique.map { it.toString().toByteArray(Charsets.UTF_8).size }
        require(lengths.all { it <= 4096 } && lengths.sum() <= 256 * 1024) { str(R.string.import_ids_long) }
        require(unique.all { it.scheme == "content" && !it.authority.isNullOrBlank() }) { str(R.string.share_pick_how) }
        return unique
    }

    private val reserved = setOf("CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$") +
        (1..9).flatMap { listOf("COM$it", "LPT$it") } + listOf("COM¹", "COM²", "COM³", "LPT¹", "LPT²", "LPT³")
    fun safeName(raw: String?, mime: String?, used: MutableSet<String>): String {
        val original = raw.orEmpty()
        val end = original.offsetByCodePoints(0, minOf(256, original.codePointCount(0, original.length)))
        var name = Normalizer.normalize(original.substring(0, end), Normalizer.Form.NFC)
            .filterNot { it.isISOControl() || it in '\u202a'..'\u202e' || it in '\u2066'..'\u2069' || it == '\u200e' || it == '\u200f' || it == '\u061c' }
            .map { if (it in "/\\:*?\"<>|") '_' else it }.joinToString("").trim().trim('.').ifBlank { str(R.string.import_file) }
        if (name.substringBefore('.').trimEnd().uppercase(Locale.ROOT) in reserved) name = "_$name"
        if (!name.contains('.')) name += when (mime?.lowercase(Locale.ROOT)) {
            "image/jpeg" -> ".jpg"; "image/png" -> ".png"; "image/heic" -> ".heic"; "image/webp" -> ".webp"
            "video/mp4" -> ".mp4"; "application/pdf" -> ".pdf"; "text/plain" -> ".txt"; else -> ""
        }
        while (name.toByteArray(Charsets.UTF_8).size > 180) {
            val dot = name.lastIndexOf('.').takeIf { it > 1 && name.length - it <= 16 } ?: name.length
            val base = name.substring(0, dot).dropLast(if (Character.isLowSurrogate(name[dot - 1])) 2 else 1)
            name = base + name.substring(dot)
        }
        val dot = name.lastIndexOf('.').takeIf { it > 0 } ?: name.length
        var candidate = name
        var n = 2
        while (!used.add(candidate.lowercase(Locale.ROOT))) candidate = name.substring(0, dot) + " (${n++})" + name.substring(dot)
        return candidate
    }

    data class Source(val uri: Uri, val name: String, val size: Long?)
    fun describe(context: Context, uri: Uri, used: MutableSet<String>): Source {
        checkedUris(listOf(uri))
        val resolver = context.contentResolver
        val mime = resolver.getType(uri)
        require(mime != "vnd.android.document/directory") { str(R.string.import_folder) }
        var name: String? = null
        var size: Long? = null
        resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { cursor ->
            if (cursor.moveToFirst()) {
                val nameIndex = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                if (nameIndex >= 0 && !cursor.isNull(nameIndex)) name = cursor.getString(nameIndex)
                val sizeIndex = cursor.getColumnIndex(OpenableColumns.SIZE)
                if (sizeIndex >= 0 && !cursor.isNull(sizeIndex)) size = cursor.getLong(sizeIndex).takeIf { it >= 0 }
            }
        }
        require(size == null || size!! <= MAX_FILE_BYTES) { str(R.string.import_limit) }
        return Source(uri, safeName(name, mime, used), size)
    }

    suspend fun stage(context: Context, source: Source, batch: File, already: Long, progress: (Long) -> Unit): Pair<File, Long> {
        val job = currentCoroutineContext()
        return runInterruptible(Dispatchers.IO) {
            require(batch.usableSpace > RESERVE + (source.size ?: 0)) { str(R.string.import_no_space_prepare) }
            val temporary = File(batch, ".import-" + java.util.UUID.randomUUID())
            val target = File(batch, source.name)
            check(!target.exists() && temporary.createNewFile()) { str(R.string.import_prepare_failed) }
            try {
                var count = 0L
                context.contentResolver.openInputStream(source.uri)?.use { input ->
                    FileOutputStream(temporary).use { output ->
                        val bytes = ByteArray(64 * 1024)
                        while (true) {
                            job.ensureActive()
                            val length = input.read(bytes)
                            if (length < 0) break
                            if (length == 0) continue
                            count += length
                            require(count <= MAX_FILE_BYTES && already + count <= MAX_BATCH_BYTES) { str(R.string.import_limit) }
                            require(batch.usableSpace > RESERVE + length) { str(R.string.import_no_space) }
                            output.write(bytes, 0, length)
                            progress(count)
                        }
                        output.fd.sync()
                    }
                } ?: error(str(R.string.import_read_failed))
                require(source.size == null || source.size == count) { str(R.string.import_size_changed) }
                check(temporary.renameTo(target)) { str(R.string.import_prepare_failed) }
                target to count
            } catch (error: Throwable) { temporary.delete(); target.delete(); throw error }
        }
    }
}
