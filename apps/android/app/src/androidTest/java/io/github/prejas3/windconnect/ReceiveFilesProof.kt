package io.github.prejas3.windconnect

import android.os.Bundle
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.AndroidComposeTestRule
import androidx.test.ext.junit.rules.ActivityScenarioRule
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import java.io.File
import java.security.MessageDigest
import org.json.JSONArray
import org.json.JSONObject

object ReceiveFilesProof {
    fun run(compose: AndroidComposeTestRule<ActivityScenarioRule<MainActivity>, MainActivity>) = runBlocking {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val app = compose.activity.application as WindApplication
        val core = app.connections.acquire()
        try {
            withTimeout(15_000) { FileReceiving.ui.first { it.ready } }
            assertTrue(ReceiveFilesService.instance != null || ConnectionService.foregroundOwner)
            if (InstrumentationRegistry.getArguments().getString("receiveLifecycleOnly") == "true") {
                withContext(Dispatchers.Main) { FileReceiving.setEnabled(app, false) }
                withTimeout(10_000) { FileReceiving.ui.first { !it.ready && !it.enabled } }
                withContext(Dispatchers.Main) { FileReceiving.setEnabled(app, true) }
                withTimeout(15_000) { FileReceiving.ui.first { it.ready } }
                compose.activityRule.scenario.moveToState(androidx.lifecycle.Lifecycle.State.CREATED)
                withTimeout(15_000) { while (ReceiveFilesService.instance != null || FileReceiving.ui.value.ready) delay(50) }
                compose.activityRule.scenario.moveToState(androidx.lifecycle.Lifecycle.State.RESUMED)
                withTimeout(15_000) { FileReceiving.ui.first { it.ready } }
                withContext(Dispatchers.Main) { assertTrue(ConnectionService.start(app, enable = true)) }
                withTimeout(15_000) { while (!ConnectionService.foregroundOwner || ReceiveFilesService.instance != null) delay(50) }
                compose.activityRule.scenario.moveToState(androidx.lifecycle.Lifecycle.State.CREATED)
                assertTrue("existing foreground owner maintains background receiving", FileReceiving.ui.value.ready)
                assertTrue(ConnectionService.foregroundOwner)
                compose.activityRule.scenario.moveToState(androidx.lifecycle.Lifecycle.State.RESUMED)
                withContext(Dispatchers.Main) { ConnectionService.stop(app) }
                withTimeout(15_000) { while (ConnectionService.foregroundOwner) delay(50) }
                instrumentation.sendStatus(0, Bundle().apply { putString("receiveFilesProof", "true"); putString("receiveLifecycleProof", "true") })
                return@runBlocking
            }
            instrumentation.sendStatus(0, Bundle().apply { putString("receiveReady", "true") })
            val receipts = withTimeout(600_000) { core.receivedFiles.first { it.size == 7 } }
            val report = JSONArray()
            for (receipt in receipts) {
                val uri = core.receivedUri(receipt.key)!!
                val hash = MessageDigest.getInstance("SHA-256")
                var size = 0L
                app.contentResolver.openInputStream(uri)!!.use { input ->
                    val buffer = ByteArray(1024 * 1024)
                    while (true) { val n = input.read(buffer); if (n < 0) break; size += n; hash.update(buffer, 0, n) }
                }
                assertEquals(receipt.size, size)
                assertTrue(receipt.folder.startsWith("Download/WindConnect/"))
                report.put(JSONObject().put("name", receipt.name).put("folder", receipt.folder).put("size", size).put("uri", uri).put("sha256", hash.digest().joinToString("") { "%02x".format(it) }))
            }
            assertTrue(receipts.any { it.size == 1024L * 1024 * 1024 })
            File(app.cacheDir, "wn120-received.json").writeText(report.toString(2))
            withTimeout(10_000) { while (core.incomingActive()) delay(50) }
            assertTrue("private verified copies released after public commit", File(app.noBackupFilesDir, "file-engine/inbox").walkTopDown().none { it.isFile })
            instrumentation.sendStatus(0, Bundle().apply { putString("receiveFilesProof", "true") })
        } finally { app.connections.release(core) }
    }
}
