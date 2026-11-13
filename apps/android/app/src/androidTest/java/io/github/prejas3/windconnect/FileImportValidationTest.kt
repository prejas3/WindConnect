package io.github.prejas3.windconnect

import android.content.ClipData
import android.content.Intent
import android.net.Uri
import org.junit.Assert.*
import org.junit.Test

class FileImportValidationTest {
    @Test fun singleAndMultipleSharesUseOnlyBoundedContentUris() {
        val uri = Uri.parse("content://fixture/photo")
        val single = Intent(Intent.ACTION_SEND).putExtra(Intent.EXTRA_STREAM, uri)
        single.clipData = ClipData.newRawUri("same URI", uri)
        assertEquals(listOf(uri), FileImport.uris(single))
        assertEquals(listOf(uri), FileImport.uris(Intent(Intent.ACTION_SEND_MULTIPLE).putParcelableArrayListExtra(Intent.EXTRA_STREAM, arrayListOf(uri, uri))))
        for (bad in listOf("file:///data/private", "https://example.com/file", "content:/no-authority"))
            assertThrows(IllegalArgumentException::class.java) { FileImport.checkedUris(listOf(Uri.parse(bad))) }
        assertThrows(IllegalArgumentException::class.java) { FileImport.checkedUris((0..100).map { Uri.parse("content://fixture/$it") }) }
        assertThrows(IllegalArgumentException::class.java) { FileImport.checkedUris(listOf(Uri.parse("content://fixture/" + "x".repeat(5000)))) }
        assertThrows(IllegalArgumentException::class.java) { FileImport.uris(Intent(Intent.ACTION_SEND).putExtra(Intent.EXTRA_TEXT, "text only")) }
        assertThrows(IllegalStateException::class.java) { FileImport.uris(Intent(Intent.ACTION_VIEW).setData(uri)) }
        assertTrue(FileImport.uris(Intent().putExtra(Intent.EXTRA_STREAM, uri)).isEmpty())
    }
}
