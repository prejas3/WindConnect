package io.github.prejas3.windconnect

import android.os.Bundle
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.view.accessibility.AccessibilityNodeInfo
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.AndroidComposeTestRule
import androidx.lifecycle.Lifecycle
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.test.ext.junit.rules.ActivityScenarioRule
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.core.ConnectionState
import kotlinx.coroutines.*
import org.junit.Assert.*
import java.io.File

/** CCS/1 CHECK:real external-UID grants/share sheet, actual picker, pipeline and FGS with notifications refused. */
object FilesProof {
    fun run(compose: AndroidComposeTestRule<ActivityScenarioRule<MainActivity>, MainActivity>) = runBlocking {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val app = compose.activity.application as WindApplication
        fun shell(command: String) = ParcelFileDescriptor.AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(command)).use { String(it.readBytes()) }
        assertEquals("1", shell("getprop ro.kernel.qemu").trim())
        val core = app.connections.acquire()
        val cancelOnly = InstrumentationRegistry.getArguments().getString("filesCancelOnly") == "true"
        val appearance = app.getSharedPreferences("appearance", android.content.Context.MODE_PRIVATE)
        val oldAppearance = appearance.all
        fun restoreAppearance() {
            val edit = appearance.edit().clear()
            oldAppearance.forEach { (key, value) -> when (value) { is String -> edit.putString(key, value); is Boolean -> edit.putBoolean(key, value) } }
            edit.commit()
        }
        fun clickSystemText(text: String): Boolean {
            val root = instrumentation.uiAutomation.rootInActiveWindow ?: return false
            for (node in root.findAccessibilityNodeInfosByText(text)) {
                var target: AccessibilityNodeInfo? = node
                repeat(5) {
                    if (target?.isClickable == true && target?.performAction(AccessibilityNodeInfo.ACTION_CLICK) == true) return true
                    target = target?.parent
                }
            }
            return false
        }
        fun source(case: String, chooser: Boolean = true) {
            shell("am start -W -n ${app.packageName}.test/io.github.prejas3.windconnect.FileShareSourceActivity --es case $case --ez chooser $chooser")
        }
        fun waitShare(chooser: Boolean) {
            val deadline = SystemClock.elapsedRealtime() + 15_000
            while (compose.onAllNodesWithTag("shareFiles").fetchSemanticsNodes(atLeastOneRootRequired = false).isEmpty()) {
                assertTrue("share target must open", SystemClock.elapsedRealtime() < deadline)
                if (chooser && !clickSystemText("Wyślij na komputer")) clickSystemText("WindConnect")
                Thread.sleep(100)
            }
        }
        fun closeShare() { compose.onNodeWithText("Zamknij").performScrollTo().performClick() }
        fun awaitComplete(before: Set<String>, count: Int) {
            runBlocking { withTimeout(90_000) {
                while (true) {
                    val rows = core.files.value.filter { it.id !in before }
                    if (rows.size == count && rows.all { it.state == "done" } && !FileSending.status.value.busy) break
                    delay(50)
                }
            } }
            assertEquals(count, FileSending.status.value.completed)
            assertTrue(FileSending.status.value.message.startsWith("Wysłano"))
            assertTrue("copies removed after confirmed transfer", core.outbox.listFiles().orEmpty().none { it.name.startsWith("batch-") })
        }
        try {
            assertTrue(core.state.value is ConnectionState.Connected)
            assertTrue(core.filesAccepted.value)
            compose.onNodeWithTag("homeSend").assertIsEnabled()
            for ((case, count, mode) in listOf(Triple("images", 20, "Light"), Triple("mixed", 4, "Dark"),
                Triple("unknown", 1, "Light"), Triple("unsafe", 1, "Dark")).filter { !cancelOnly }) {
                val beforeShare = core.files.value.map { it.id }.toSet()
                appearance.edit().putString("theme", mode).putBoolean("dynamic", false).commit()
                source(case)
                waitShare(true)
                compose.waitUntil(10_000) { compose.onNodeWithTag("confirmSendFiles").fetchSemanticsNode().config.contains(androidx.compose.ui.semantics.SemanticsProperties.Disabled).not() }
                compose.onNodeWithTag("selectedFileCount").assertTextEquals("Wybrane pliki: $count")
                assertEquals("no transfer before explicit consent", beforeShare, core.files.value.map { it.id }.toSet())
                assertTrue("no payload staged before consent", core.outbox.listFiles().orEmpty().none { it.name.startsWith("batch-") })
                compose.onNodeWithTag("confirmSendFiles").performScrollTo()
                if (case in listOf("images", "mixed")) {
                    val image = compose.onNodeWithTag("shareFiles").captureToImage().asAndroidBitmap()
                    File(app.cacheDir, "wn40-$mode-150.png").outputStream().use { image.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
                    image.recycle()
                }
                val before = core.files.value.map { it.id }.toSet()
                compose.onNodeWithTag("confirmSendFiles").performClick()
                if (case == "images") {
                    compose.waitUntil(5_000) { FileSending.status.value.busy }
                    compose.waitUntil(10_000) { core.files.value.any { it.id !in before } }
                    instrumentation.sendStatus(0, Bundle().apply { putString("filesTransferStarted", "true") })
                    closeShare()
                    compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
                    awaitComplete(before, count)
                    compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
                } else { awaitComplete(before, count); closeShare() }
            }
            for (case in listOf("denied", "oversize").filter { !cancelOnly }) {
                val before = core.files.value.size
                source(case, false); waitShare(false)
                compose.waitUntil(5_000) { compose.onAllNodesWithTag("fileShareError").fetchSemanticsNodes().isNotEmpty() }
                compose.onNodeWithTag("confirmSendFiles").assertIsNotEnabled()
                assertEquals(before, core.files.value.size)
                closeShare()
            }
            source("cancel", false); waitShare(false)
            compose.waitUntil(5_000) { !compose.onNodeWithTag("confirmSendFiles").fetchSemanticsNode().config.contains(androidx.compose.ui.semantics.SemanticsProperties.Disabled) }
            compose.onNodeWithTag("confirmSendFiles").performScrollTo().performClick()
            compose.waitUntil(5_000) { FileSending.status.value.busy }
            compose.waitUntil(5_000) { FileSending.status.value.message.startsWith("Przygotowano") }
            compose.onNodeWithTag("cancelFiles").assertIsDisplayed().performClick()
            try { compose.waitUntil(10_000) { !FileSending.status.value.busy } }
            catch (failure: Throwable) { throw AssertionError("cancel phase=${FileSending.debugPhase} status=${FileSending.status.value}", failure) }
            assertTrue(FileSending.status.value.message.contains("Anulowano"))
            compose.waitUntil(5_000) {
                @Suppress("DEPRECATION")
                val services = app.getSystemService(android.app.ActivityManager::class.java).getRunningServices(100)
                services.none { it.service.className == SendFilesService::class.java.name }
            }
            assertTrue(core.outbox.listFiles().orEmpty().none { it.name.startsWith("batch-") })
            closeShare()
            if (cancelOnly) {
                instrumentation.sendStatus(0, Bundle().apply { putString("filesProof", "true"); putString("filesCancelProof", "true") })
                return@runBlocking
            }
            source("picker")
            compose.onNodeWithTag("homeSend").performScrollTo().performClick()
            val deadline = SystemClock.elapsedRealtime() + 15_000
            while (!clickSystemText("wn40-picker.pdf")) {
                assertTrue("real picker must show fixture document", SystemClock.elapsedRealtime() < deadline)
                clickSystemText("Downloads"); Thread.sleep(150)
            }
            Thread.sleep(150)
            if (compose.onAllNodesWithTag("shareFiles").fetchSemanticsNodes(atLeastOneRootRequired = false).isEmpty()) clickSystemText("Open")
            waitShare(false)
            compose.waitUntil(5_000) { !compose.onNodeWithTag("confirmSendFiles").fetchSemanticsNode().config.contains(androidx.compose.ui.semantics.SemanticsProperties.Disabled) }
            val before = core.files.value.map { it.id }.toSet()
            compose.onNodeWithTag("confirmSendFiles").performScrollTo().performClick()
            awaitComplete(before, 1); closeShare()
            instrumentation.sendStatus(0, Bundle().apply { putString("filesProof", "true"); putString("filesDelivered", "27") })
        } finally {
            if (FileSending.status.value.busy) SendFilesService.cancel(app)
            source("cleanup")
            restoreAppearance()
            app.connections.release(core)
        }
    }
}
