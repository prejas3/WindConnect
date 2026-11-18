package io.github.prejas3.windconnect.core

import android.content.ContentProvider
import android.content.ContentResolver
import android.content.ContentValues
import android.content.Context
import android.content.res.AssetFileDescriptor
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.os.Bundle
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import io.github.prejas3.windconnect.bindings.ClipboardImageInterface
import java.io.FileNotFoundException
import java.io.IOException

/**
 * CCS/1 M:received clipboard images (WN-36, PROTOCOL.md §9.3) | SEC:not exported; access only through the clip's URI grant;
 * memory only, the latest image only; a newer item, a restart or a failed transfer makes a paste fail, never a mix.
 *
 * `content://<app>.clipimage/<id>`: a preview (ContentResolver.EXTRA_SIZE, e.g. the clipboard overlay or a keyboard)
 * gets the thumbnail at once; a paste gets the full PNG streamed as it arrives.
 */
class ClipImageProvider : ContentProvider() {
    override fun onCreate() = true

    override fun getType(uri: Uri): String? = current(uri)?.let { PNG }

    override fun getStreamTypes(uri: Uri, mimeTypeFilter: String): Array<String>? =
        if (current(uri) != null && ClipDescriptionMatch.matches(PNG, mimeTypeFilter)) arrayOf(PNG) else null

    override fun query(uri: Uri, projection: Array<out String>?, selection: String?, selectionArgs: Array<out String>?, sortOrder: String?): Cursor? {
        val image = current(uri) ?: return null
        val columns = projection?.filter { it == OpenableColumns.DISPLAY_NAME || it == OpenableColumns.SIZE }?.toTypedArray()
            ?: arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)
        return MatrixCursor(columns, 1).apply {
            addRow(columns.map { if (it == OpenableColumns.SIZE) image.size().toLong() else context!!.getString(R.string.clip_image_file_name) })
        }
    }

    override fun openTypedAssetFile(uri: Uri, mimeTypeFilter: String, opts: Bundle?): AssetFileDescriptor {
        val image = current(uri) ?: throw FileNotFoundException("no such clipboard image")
        if (!ClipDescriptionMatch.matches(PNG, mimeTypeFilter)) throw FileNotFoundException("only $PNG")
        // A preview asks with a size: the thumbnail, never pasted as the image (WN-36 review point 1).
        if (opts?.containsKey(ContentResolver.EXTRA_SIZE) == true) {
            return AssetFileDescriptor(pipe(image.id()) { out, _ -> out.write(image.thumbnail()) }, 0, AssetFileDescriptor.UNKNOWN_LENGTH)
        }
        return AssetFileDescriptor(openFile(uri, "r"), 0, AssetFileDescriptor.UNKNOWN_LENGTH)
    }

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        if (mode != "r") throw FileNotFoundException("read only")
        val image = current(uri) ?: throw FileNotFoundException("no such clipboard image")
        return pipe(image.id()) { out, isCurrent ->
            var offset = 0uL
            while (true) {
                if (!isCurrent()) throw IOException("replaced")
                // Blocks until bytes arrive; fails when replaced, broken off or stalled for 10 s.
                val part = try { image.read(offset, CHUNK) } catch (error: Exception) { throw IOException("image transfer failed", error) }
                if (part.isEmpty()) break
                out.write(part)
                offset += part.size.toULong()
            }
        }
    }

    /** A pipe a worker fills; an error closes it with an error, so the reader fails instead of getting a partial image. */
    private fun pipe(id: String, fill: (ParcelFileDescriptor.AutoCloseOutputStream, () -> Boolean) -> Unit): ParcelFileDescriptor {
        val (reader, writer) = ParcelFileDescriptor.createReliablePipe()
        Thread({
            val out = ParcelFileDescriptor.AutoCloseOutputStream(writer)
            try {
                fill(out) { synchronized(lock) { latest?.id() == id } }
                out.close()
            } catch (error: Exception) {
                try { writer.closeWithError(error.message ?: "image unavailable") } catch (_: IOException) {}
            }
        }, "clipimage-$id").apply { isDaemon = true }.start()
        return reader
    }

    override fun insert(uri: Uri, values: ContentValues?): Uri? = null
    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?) = 0
    override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<out String>?) = 0

    companion object {
        const val PNG = "image/png"
        private const val CHUNK: UInt = 65_536u
        private val lock = Any()
        private var latest: ClipboardImageInterface? = null

        fun authority(context: Context) = context.packageName + ".clipimage"
        fun uri(context: Context, image: ClipboardImageInterface): Uri = Uri.Builder().scheme("content").authority(authority(context)).appendPath(image.id()).build()
        fun isOurs(context: Context, uri: Uri?) = uri?.authority == authority(context)

        /** The image a paste of the clipboard gets; replaces the previous one. */
        fun publish(image: ClipboardImageInterface?) = synchronized(lock) { latest = image }

        private fun current(uri: Uri): ClipboardImageInterface? = synchronized(lock) {
            latest?.takeIf { uri.pathSegments.singleOrNull() == it.id() }
        }
    }
}

/** ClipDescription.compareMimeTypes without needing a ClipDescription. */
internal object ClipDescriptionMatch {
    fun matches(type: String, filter: String): Boolean {
        if (filter == "*/*") return true
        val (t, s) = type.split('/', limit = 2).let { it[0] to it.getOrElse(1) { "" } }
        val (ft, fs) = filter.split('/', limit = 2).let { it[0] to it.getOrElse(1) { "" } }
        return ft == t && (fs == "*" || fs == s)
    }
}
