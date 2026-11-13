package io.github.prejas3.windconnect

import android.provider.MediaStore
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.bindings.*
import io.github.prejas3.windconnect.core.IncomingFiles
import org.junit.Assert.*
import org.junit.Test
import org.junit.Assume.assumeTrue
import java.io.File
import java.util.UUID

class IncomingFilesTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private class Control : IncomingControl(NoHandle) {
        var cancelled = false
        override fun isCancelled() = cancelled
    }
    private class Crash : Error()
    private fun source(name: String): IncomingFile {
        val id = UUID.randomUUID().toString().replace("-", "")
        val root = File(context.noBackupFilesDir, "file-engine/inbox").apply { mkdirs() }
        val file = File(root, id).apply { writeBytes(ByteArray(131_079) { (it % 251).toByte() }) }
        return IncomingFile("ab".repeat(32), id, name, file.absolutePath, file.length().toULong())
    }
    private fun pendingCount(): Int {
        @Suppress("DEPRECATION")
        val uri = MediaStore.setIncludePending(MediaStore.Downloads.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY))
        return context.contentResolver.query(uri, arrayOf(MediaStore.MediaColumns._ID), "${MediaStore.MediaColumns.OWNER_PACKAGE_NAME}=? AND ${MediaStore.MediaColumns.IS_PENDING}=1", arrayOf(context.packageName), null)!!.use { it.count }
    }
    @Test fun publicFilesHaveExactBytesCollisionSafetyAndIdempotentReceipts() {
        val store = IncomingFiles(context)
        val token = UUID.randomUUID().toString()
        val created = mutableListOf<android.net.Uri>()
        try {
            for (name in listOf("data.json", "archive.tar.gz", "noextension", "installer.apk", "data.json")) {
                val file = source("$token/folder/$name")
                val bytes = File(file.source).readBytes()
                assertEquals(FileCommitStatus.PUBLISHED, store.commit(file, Control()).status)
                val receipt = store.receipts.value.first()
                val uri = store.receiptUri(receipt.key)!!
                created.add(uri)
                assertTrue(receipt.folder.endsWith("/$token/folder/"))
                assertArrayEquals(bytes, context.contentResolver.openInputStream(uri)!!.use { it.readBytes() })
                assertEquals(FileCommitStatus.PUBLISHED, store.commit(file, Control()).status)
                assertEquals(uri, store.receiptUri(receipt.key))
                File(file.source).delete()
            }
            assertEquals(created.size, created.toSet().size)
            assertEquals(0, pendingCount())
        } finally { created.forEach { context.contentResolver.delete(it, null, null) }; store.clearHistory() }
    }
    @Test fun interruptedTransactionsRecoverWithoutDuplicateVisibleFilesOrPendingRows() {
        for (phase in listOf("inserted", "pending", "copying", "ready", "visible", "published")) {
            val file = source("recovery-${UUID.randomUUID()}/data.json")
            val bytes = File(file.source).readBytes()
            val interrupted = IncomingFiles(context) { if (it == phase) throw Crash() }
            assertThrows(Crash::class.java) { interrupted.commit(file, Control()) }
            val recovered = IncomingFiles(context)
            assertEquals(0, pendingCount())
            // A restarted sender re-verifies into a fresh private file with the same token.
            File(file.source).apply { parentFile!!.mkdirs(); writeBytes(bytes) }
            assertEquals(FileCommitStatus.PUBLISHED, recovered.commit(file, Control()).status)
            val key = "${file.peer}:${file.id}"
            val uri = recovered.receiptUri(key)!!
            assertEquals(1, recovered.receipts.value.count { it.key == key })
            assertArrayEquals(bytes, context.contentResolver.openInputStream(uri)!!.use { it.readBytes() })
            context.contentResolver.delete(uri, null, null)
            File(file.source).delete(); recovered.clearHistory()
        }
    }
    @Test fun cancellationAndNoSpaceNeverPublishPartialFiles() {
        for (noSpace in listOf(false, true)) {
            val file = source("cancel-${UUID.randomUUID()}/data.json")
            val control = Control()
            val store = IncomingFiles(context) { phase -> if (phase == "copying") {
                if (noSpace) throw android.system.ErrnoException("write", android.system.OsConstants.ENOSPC)
                control.cancelled = true
            } }
            assertEquals(if (noSpace) FileCommitStatus.NO_SPACE else FileCommitStatus.CANCELLED, store.commit(file, control).status)
            assertNull(store.receiptUri("${file.peer}:${file.id}"))
            assertEquals(0, pendingCount())
            File(file.source).delete()
        }
    }
    @Test fun movingPublishedFileDoesNotDisableFurtherReceivingOrDeleteUserData() {
        val file = source("move-${UUID.randomUUID()}/data.json")
        val store = IncomingFiles(context)
        assertEquals(FileCommitStatus.PUBLISHED, store.commit(file, Control()).status)
        val key = "${file.peer}:${file.id}"
        val uri = store.receiptUri(key)!!
        try {
            context.contentResolver.update(uri, android.content.ContentValues().apply { put(MediaStore.MediaColumns.RELATIVE_PATH, "Download/moved-${UUID.randomUUID()}/") }, null, null)
            val reopened = IncomingFiles(context)
            assertNull(reopened.receiptUri(key))
            assertEquals(file.size.toLong(), context.contentResolver.openInputStream(uri)!!.use { it.readBytes().size.toLong() })
            reopened.clearHistory()
        } finally { context.contentResolver.delete(uri, null, null); File(file.source).delete() }
    }
    @Test fun explicitReadGrantOpensExactPublishedBytesFromAnotherUid() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        fun shell(command: String) = android.os.ParcelFileDescriptor.AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(command)).use { String(it.readBytes()) }
        shell("run-as ${context.packageName}.test rm -f cache/wn120-open.json")
        val file = source("external-${UUID.randomUUID()}/data.json")
        val bytes = File(file.source).readBytes()
        val store = IncomingFiles(context)
        assertEquals(FileCommitStatus.PUBLISHED, store.commit(file, Control()).status)
        val uri = store.receiptUri("${file.peer}:${file.id}")!!
        try {
            instrumentation.runOnMainSync {
                context.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW).setDataAndType(uri, "application/json")
                    .setComponent(android.content.ComponentName(context.packageName + ".test", ReceivedFileViewerFixture::class.java.name))
                    .addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK or android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION))
            }
            val deadline = android.os.SystemClock.elapsedRealtime() + 15_000
            var opened: org.json.JSONObject? = null
            while (opened == null && android.os.SystemClock.elapsedRealtime() < deadline) {
                try { opened = org.json.JSONObject(shell("run-as ${context.packageName}.test cat cache/wn120-open.json")) } catch (_: Exception) { android.os.SystemClock.sleep(50) }
            }
            assertNotNull(opened)
            assertFalse(opened!!.has("error"))
            assertNotEquals(android.os.Process.myUid(), opened.getInt("uid"))
            assertEquals(bytes.size.toLong(), opened.getLong("size"))
            assertEquals(java.security.MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }, opened.getString("sha256"))
        } finally { context.contentResolver.delete(uri, null, null); File(file.source).delete(); store.clearHistory() }
    }
    @Test fun killAtTransactionPhase() {
        val phase = InstrumentationRegistry.getArguments().getString("killPhase")
        assumeTrue(phase != null)
        val file = source("crash-${UUID.randomUUID()}/data.json")
        File(context.cacheDir, "wn120-crash.json").writeText(org.json.JSONObject().put("peer", file.peer).put("id", file.id).put("name", file.name).put("source", file.source).put("size", file.size.toLong()).toString())
        val store = IncomingFiles(context) { current -> if (current == phase) {
            InstrumentationRegistry.getInstrumentation().sendStatus(0, android.os.Bundle().apply { putString("killReady", phase) })
            while (true) android.os.SystemClock.sleep(100)
        } }
        store.commit(file, Control())
        fail("fixture must be force-stopped at requested phase")
    }
    @Test fun recoverAfterProcessKill() {
        assumeTrue(InstrumentationRegistry.getArguments().getString("recoverKilled") == "true")
        val data = org.json.JSONObject(File(context.cacheDir, "wn120-crash.json").readText())
        val file = IncomingFile(data.getString("peer"), data.getString("id"), data.getString("name"), data.getString("source"), data.getLong("size").toULong())
        val store = IncomingFiles(context)
        assertEquals(0, pendingCount())
        File(file.source).apply { parentFile!!.mkdirs(); writeBytes(ByteArray(file.size.toInt()) { (it % 251).toByte() }) }
        assertEquals(FileCommitStatus.PUBLISHED, store.commit(file, Control()).status)
        val key = "${file.peer}:${file.id}"
        val receipt = store.receipts.value.first { it.key == key }
        val uri = store.receiptUri(key)!!
        assertArrayEquals(File(file.source).readBytes(), context.contentResolver.openInputStream(uri)!!.use { it.readBytes() })
        val collection = MediaStore.Downloads.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        context.contentResolver.query(collection, arrayOf(MediaStore.MediaColumns._ID), "${MediaStore.MediaColumns.RELATIVE_PATH}=? AND ${MediaStore.MediaColumns.OWNER_PACKAGE_NAME}=?", arrayOf(receipt.folder, context.packageName), null)!!.use { assertEquals("no duplicate visible files", 1, it.count) }
        context.contentResolver.delete(uri, null, null); File(file.source).delete(); store.clearHistory()
    }
}
