package io.github.prejas3.windconnect

import android.app.NotificationManager
import android.content.ClipData
import android.content.ClipboardManager
import android.os.Bundle
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.view.accessibility.AccessibilityNodeInfo
import androidx.compose.ui.test.*
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.junit4.AndroidComposeTestRule
import androidx.test.ext.junit.rules.ActivityScenarioRule
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.core.ConnectionState
import kotlinx.coroutines.*
import org.junit.Assert.*

/** CCS/1 CHECK:real posted notification PendingIntents -> focused clipboard / system picker -> trusted fixture. */
object ConnectionNotificationProof {
    fun run(compose: AndroidComposeTestRule<ActivityScenarioRule<MainActivity>, MainActivity>) = runBlocking {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.uiAutomation.serviceInfo = instrumentation.uiAutomation.serviceInfo.apply {
            flags = flags or android.accessibilityservice.AccessibilityServiceInfo.FLAG_REPORT_VIEW_IDS or android.accessibilityservice.AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS
        }
        val app = compose.activity.application as WindApplication
        val manager = app.getSystemService(NotificationManager::class.java)
        val core = app.connections.acquire()
        val appearance = app.getSharedPreferences("appearance", android.content.Context.MODE_PRIVATE)
        val oldTheme = appearance.getString("theme", "System")
        val oldDynamic = appearance.getBoolean("dynamic", true)
        fun shell(command: String) = ParcelFileDescriptor.AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(command)).use { String(it.readBytes()) }
        fun await(message: String, condition: () -> Boolean) {
            val until = SystemClock.elapsedRealtime() + 20_000
            while (!condition()) { assertTrue(message, SystemClock.elapsedRealtime() < until); Thread.sleep(100) }
        }
        fun notification() = manager.activeNotifications.firstOrNull { it.id == ConnectionService.ID }?.notification
        fun actions() = notification()?.actions?.map { it.title.toString() }.orEmpty()
        fun click(text: String): Boolean {
            val nodes = mutableListOf<AccessibilityNodeInfo>()
            fun visit(node: AccessibilityNodeInfo?) {
                if (node == null) return
                if (node.text?.toString()?.contains(text, ignoreCase = true) == true || node.contentDescription?.toString()?.contains(text, ignoreCase = true) == true) nodes.add(node)
                repeat(node.childCount) { visit(node.getChild(it)) }
            }
            instrumentation.uiAutomation.windows.forEach { visit(it.root) }
            for (node in nodes) {
                var target: AccessibilityNodeInfo? = node
                repeat(6) {
                    if (target?.isClickable == true && target?.performAction(AccessibilityNodeInfo.ACTION_CLICK) == true) return true
                    target = target?.parent
                }
            }
            return false
        }
        fun tapAction(label: String) {
            shell("cmd statusbar expand-notifications")
            await("notification action visible: $label") {
                if (click(label)) true else {
                    click("Expand")
                    false
                }
            }
        }
        try {
            assertEquals("1", shell("getprop ro.kernel.qemu").trim())
            assertTrue(core.state.value is ConnectionState.Connected)
            instrumentation.runOnMainSync { assertTrue(ConnectionService.start(app, enable = true)) }
            await("three connected actions") { actions() == listOf("Wyślij schowek", "Wyślij plik", "Zatrzymaj") }
            val first = notification()!!
            assertNotNull(first.contentIntent)
            assertTrue(first.publicVersion.actions.isNullOrEmpty())
            first.actions.take(2).forEach { assertTrue(it.actionIntent.isImmutable); if (android.os.Build.VERSION.SDK_INT >= 31) assertTrue(it.isAuthenticationRequired) }
            assertNotEquals(first.actions[0].actionIntent, first.actions[1].actionIntent)
            core.setClipboardEnabled(false)
            instrumentation.runOnMainSync {
                app.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("WN97", "WN97-NOTIFICATION-CLIPBOARD"))
            }
            val before = core.clipboardUi.value.delivered
            tapAction("Wyślij schowek")
            await("manual clipboard applied") { core.clipboardUi.value.delivered > before }
            await("main focus restored") { compose.activity.hasWindowFocus() }
            shell("am start -W -n ${app.packageName}.test/io.github.prejas3.windconnect.FileShareSourceActivity --es case picker")
            tapAction("Wyślij plik")
            await("DocumentsUI picker") { click("wn40-picker.pdf") || run { click("Downloads"); false } }
            await("file confirmation screen") {
                click("Open")
                compose.onAllNodesWithTag("shareFiles").fetchSemanticsNodes(atLeastOneRootRequired = false).isNotEmpty()
            }
            val fileAction = first.actions[1].actionIntent
            instrumentation.runOnMainSync { fileAction.send(); fileAction.send() }
            compose.onNodeWithTag("selectedFileCount").assertTextEquals("Wybrane pliki: 1")
            compose.waitUntil(10_000) { !compose.onNodeWithTag("confirmSendFiles").fetchSemanticsNode().config.contains(androidx.compose.ui.semantics.SemanticsProperties.Disabled) }
            val ids = core.files.value.map { it.id }.toSet()
            compose.onNodeWithTag("confirmSendFiles").performScrollTo().performClick()
            await("file delivered") { core.files.value.any { it.id !in ids && it.state == "done" } && !FileSending.status.value.busy }
            compose.onNodeWithTag("closeShare").performClick()
            val completedIds = core.files.value.map { it.id }.toSet()
            for (mode in listOf("Light", "Dark")) {
                app.getSharedPreferences("appearance", android.content.Context.MODE_PRIVATE).edit().putString("theme", mode).putBoolean("dynamic", false).commit()
                instrumentation.runOnMainSync { fileAction.send() }
                await("picker reopened after close") { instrumentation.uiAutomation.rootInActiveWindow?.packageName?.toString()?.contains("documentsui") == true }
                shell("input keyevent 4")
                await("cancel returns to own screen") { compose.onAllNodesWithTag("shareFiles").fetchSemanticsNodes(atLeastOneRootRequired = false).isNotEmpty() }
                compose.onNodeWithTag("selectedFileCount").assertTextEquals("Wybrane pliki: 0")
                compose.onNodeWithTag("confirmSendFiles").performScrollTo().assertIsNotEnabled()
                assertEquals("cancel sends nothing", completedIds, core.files.value.map { it.id }.toSet())
                val bitmap = compose.onNodeWithTag("shareFiles").captureToImage().asAndroidBitmap()
                java.io.File(app.cacheDir, "wn97-$mode-150.png").outputStream().use { bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }; bitmap.recycle()
                compose.onNodeWithTag("closeShare").performClick()
            }
            // CCS/1 CHECK:screen lifecycle with an active batch; transfer correctness is checked above.
            val terminal = FileSending.status.value
            try {
                FileSending.update(FileSendStatus(busy = true, message = "Test aktywnej wysyłki", count = 1, target = "Test PC"))
                instrumentation.runOnMainSync { fileAction.send(); fileAction.send() }
                await("active batch screen instead of picker") { compose.onAllNodesWithTag("cancelFiles").fetchSemanticsNodes(atLeastOneRootRequired = false).isNotEmpty() }
                compose.onNodeWithTag("cancelFiles").assertHasClickAction()
                compose.onNodeWithTag("pickFiles").assertDoesNotExist()
                compose.onNodeWithTag("closeShare").performClick()
            } finally { FileSending.update(terminal) }
            instrumentation.sendStatus(0, Bundle().apply { putString("withdrawFiles", "true") })
            await("capability withdrawal updates notification") { actions() == listOf("Wyślij schowek", "Zatrzymaj") }
            instrumentation.sendStatus(0, Bundle().apply { putString("restoreFiles", "true") })
            await("capability reenabled") { actions() == listOf("Wyślij schowek", "Wyślij plik", "Zatrzymaj") }
            instrumentation.runOnMainSync { ConnectionService.preferences(app).edit().putBoolean("enabled", false).commit() }
            core.disconnect()
            await("no send actions disconnected") { actions() == listOf("Otwórz urządzenie", "Połącz ponownie", "Zatrzymaj") }
            instrumentation.sendStatus(0, Bundle().apply { putString("connectionNotificationProof", "true") })
        } catch (failure: Throwable) {
            val tree = StringBuilder()
            fun describe(node: AccessibilityNodeInfo?) {
                if (node == null) return
                tree.appendLine("${node.viewIdResourceName} text=${node.text} desc=${node.contentDescription} clickable=${node.isClickable}")
                repeat(node.childCount) { describe(node.getChild(it)) }
            }
            describe(instrumentation.uiAutomation.rootInActiveWindow)
            java.io.File(app.cacheDir, "wn97-failure-tree.txt").writeText(tree.toString())
            instrumentation.uiAutomation.takeScreenshot()?.let { bitmap ->
                java.io.File(app.cacheDir, "wn97-failure.png").outputStream().use { bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }; bitmap.recycle()
            }
            java.io.File(app.cacheDir, "wn97-notifications.txt").writeText(shell("dumpsys notification --noredact"))
            throw failure
        } finally {
            appearance.edit().putString("theme", oldTheme).putBoolean("dynamic", oldDynamic).commit()
            instrumentation.runOnMainSync { ConnectionService.stop(app) }
            app.connections.release(core)
        }
    }
}
