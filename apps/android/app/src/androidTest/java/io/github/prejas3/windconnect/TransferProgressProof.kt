package io.github.prejas3.windconnect

import android.os.Bundle
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import androidx.compose.ui.test.*
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.junit4.AndroidComposeTestRule
import androidx.test.ext.junit.rules.ActivityScenarioRule
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first
import org.junit.Assert.*

/** CCS/1 CHECK:5 real URI sources,2 receiver-confirmed before injected IO break, retry sends only remaining3. */
object TransferProgressProof {
    fun run(compose: AndroidComposeTestRule<ActivityScenarioRule<MainActivity>, MainActivity>) = runBlocking {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val app = compose.activity.application as WindApplication
        val core = app.connections.acquire()
        val appearance = app.getSharedPreferences("appearance", android.content.Context.MODE_PRIVATE)
        val oldTheme = appearance.getString("theme", "System")
        val oldDynamic = appearance.getBoolean("dynamic", true)
        appearance.edit().putString("theme", InstrumentationRegistry.getArguments().getString("progressTheme") ?: "Light").putBoolean("dynamic", false).commit()
        fun shell(command: String) = ParcelFileDescriptor.AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(command)).use { String(it.readBytes()) }
        fun source(kind: String) = shell("am start -W -n ${app.packageName}.test/io.github.prejas3.windconnect.FileShareSourceActivity --es case $kind --ez chooser false")
        fun ready() = compose.waitUntil(15_000) { compose.onAllNodesWithTag("confirmSendFiles").fetchSemanticsNodes(atLeastOneRootRequired = false).any { !it.config.contains(androidx.compose.ui.semantics.SemanticsProperties.Disabled) } }
        try {
            val before = core.files.value.map { it.id }.toSet()
            source("retry"); ready()
            compose.onNodeWithTag("confirmSendFiles").performScrollTo().performClick()
            withTimeout(15_000) { core.files.first { rows -> rows.count { it.id !in before && it.state == "done" } == 2 } }
            compose.onNodeWithTag("transferProgress").assertExists()
            assertNull("total remains unknown until all sources are imported", FileSending.status.value.total)
            val notification = app.getSystemService(android.app.NotificationManager::class.java).activeNotifications.first { it.id == 40 }.notification
            assertTrue(notification.extras.getBoolean(android.app.Notification.EXTRA_PROGRESS_INDETERMINATE))
            val image = compose.onNodeWithTag("shareFiles").captureToImage().asAndroidBitmap()
            java.io.File(app.cacheDir, "wn41-progress.png").outputStream().use { image.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }; image.recycle()
            shell("run-as ${app.packageName}.test touch cache/wn41-first-two-done")
            withTimeout(10_000) { FileSending.status.first { !it.busy && it.canRetry } }
            assertEquals(2, FileSending.status.value.completed)
            assertEquals(3, FileRetry.selection!!.uris.size)
            val target = FileRetry.selection!!.target
            compose.onNodeWithTag("retryFiles").performClick()
            ready(); compose.onNodeWithTag("selectedFileCount").assertTextEquals("Chosen files: 3")
            assertEquals(target, FileRetry.selection!!.target)
            compose.onNodeWithTag("confirmSendFiles").performScrollTo().performClick()
            withTimeout(15_000) { FileSending.status.first { !it.busy && it.completed == 3 } }
            assertEquals(5, core.files.value.count { it.id !in before && it.state == "done" })
            assertNull(FileRetry.selection)
            compose.onNodeWithTag("closeShare").performClick()
            // A missing grant is checked by opening the source before any retry confirmation.
            FileRetry.selection = RetrySelection(listOf(android.net.Uri.parse("content://${app.packageName}.test.files_fixture/denied/0")), target)
            FileSending.update(FileSendStatus(message = "Test ponawiania", canRetry = true))
            instrumentation.runOnMainSync { compose.activity.startActivity(android.content.Intent(app, ShareFilesActivity::class.java)) }
            compose.waitUntil(10_000) { compose.onAllNodesWithTag("retryFiles").fetchSemanticsNodes(atLeastOneRootRequired = false).isNotEmpty() }
            compose.onNodeWithTag("retryFiles").performClick()
            compose.waitUntil(10_000) { compose.onAllNodesWithTag("fileShareError").fetchSemanticsNodes(atLeastOneRootRequired = false).isNotEmpty() }
            compose.onNodeWithTag("confirmSendFiles").assertIsNotEnabled()
            assertEquals(5, core.files.value.count { it.id !in before && it.state == "done" })
            instrumentation.sendStatus(0, Bundle().apply { putString("transferProgressProof", "true"); putString("retryDelivered", "5") })
        } finally {
            appearance.edit().putString("theme", oldTheme).putBoolean("dynamic", oldDynamic).commit()
            FileRetry.selection = null; app.connections.release(core)
        }
    }
}
