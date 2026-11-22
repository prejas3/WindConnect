package io.github.prejas3.windconnect

import android.content.ComponentName
import android.content.Intent
import android.content.pm.ShortcutManager
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

class LauncherShortcutsTest {
    @get:Rule val compose = createEmptyComposeRule()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext

    @Test fun staticClipboardAndDynamicFeatureActionsArePublishedWithoutDuplicates() {
        val manager = context.getSystemService(ShortcutManager::class.java)
        val clipboard = manager.manifestShortcuts.single { it.id == LauncherAction.Clipboard.id }
        assertTrue(clipboard.isImmutable)
        assertEquals(LauncherAction.Clipboard.action, clipboard.intent?.action)
        assertEquals(ComponentName(context, LauncherShortcutActivity::class.java), clipboard.intent?.component)
        LauncherShortcuts.publish(context)
        LauncherShortcuts.publish(context)
        assertEquals(setOf("send_file", "touchpad"), manager.dynamicShortcuts.map { it.id }.toSet())
        manager.dynamicShortcuts.forEach {
            assertFalse(it.shortLabel.toString().contains("unavailable", ignoreCase = true))
            assertEquals(LauncherAction.entries.single { a -> a.id == it.id }.title, it.longLabel.toString())
            assertEquals(ComponentName(context, LauncherShortcutActivity::class.java), it.intent?.component)
            assertEquals(ComponentName(context, MainActivity::class.java), it.activity)
        }
    }

    /** CCS/1 CHECK:WN-116 file/touchpad shortcuts start their real screens directly (blocked here, so no picker opens), never Home. */
    @Test fun fileAndTouchpadRouteStraightToTheirFeatureScreens() {
        val started = java.util.Collections.synchronizedList(mutableListOf<Intent>())
        val monitor = object : android.app.Instrumentation.ActivityMonitor() {
            private val blocked = setOf(ShareFilesActivity::class.java.name, TouchpadActivity::class.java.name, MainActivity::class.java.name)
            override fun onStartActivity(intent: Intent): android.app.Instrumentation.ActivityResult? {
                if ((intent.component?.className ?: return null) !in blocked) return null
                started += intent
                return android.app.Instrumentation.ActivityResult(0, null)
            }
        }
        instrumentation.addMonitor(monitor)
        try {
            for (action in listOf(LauncherAction.File, LauncherAction.Touchpad)) {
                ActivityScenario.launch<LauncherShortcutActivity>(LauncherShortcuts.intent(context, action)).use {
                    assertEquals(androidx.lifecycle.Lifecycle.State.DESTROYED, it.state)
                }
            }
        } finally { instrumentation.removeMonitor(monitor) }
        val targets = started.toList()
        assertEquals(listOf(ShareFilesActivity::class.java.name, TouchpadActivity::class.java.name), targets.map { it.component?.className })
        assertEquals(ShareFilesActivity.PICK, targets[0].action)
        assertTrue(targets.none { it.component?.className == MainActivity::class.java.name })
    }

    @Test fun clipboardRoutesToFocusedSenderAndReportsOfflineWithoutHome() {
        val main = instrumentation.addMonitor(MainActivity::class.java.name, null, false)
        val sender = instrumentation.addMonitor(SendClipboardActivity::class.java.name, null, false)
        try {
            ActivityScenario.launch<LauncherShortcutActivity>(LauncherShortcuts.intent(context, LauncherAction.Clipboard)).use {
                val activity = sender.waitForActivityWithTimeout(15_000)
                assertNotNull("Shortcut must open the existing focused sender", activity)
                try {
                    compose.waitUntil(15_000) {
                        compose.onAllNodesWithText("Clipboard unavailable — check the connection to the computer").fetchSemanticsNodes().isNotEmpty()
                    }
                    compose.onNodeWithTag("computerCard").assertDoesNotExist()
                    assertEquals(0, main.hits)
                } finally { activity?.let { instrumentation.runOnMainSync { it.finish() } } }
            }
        } finally {
            instrumentation.removeMonitor(main)
            instrumentation.removeMonitor(sender)
        }
    }

    @Test fun malformedExternalActionFinishesWithoutLaunchingAnotherScreen() {
        val sender = instrumentation.addMonitor(SendClipboardActivity::class.java.name, null, false)
        try {
            val intent = Intent(context, LauncherShortcutActivity::class.java).setAction(Intent.ACTION_VIEW)
                .putExtra("action", LauncherAction.Clipboard.action).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            ActivityScenario.launch<LauncherShortcutActivity>(intent).use {
                assertEquals(androidx.lifecycle.Lifecycle.State.DESTROYED, it.state)
                assertEquals(0, sender.hits)
            }
        } finally { instrumentation.removeMonitor(sender) }
    }
}
