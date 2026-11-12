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

    @Test fun staticClipboardAndDynamicUnavailableActionsArePublishedWithoutDuplicates() {
        val manager = context.getSystemService(ShortcutManager::class.java)
        val clipboard = manager.manifestShortcuts.single { it.id == LauncherAction.Clipboard.id }
        assertTrue(clipboard.isImmutable)
        assertEquals(LauncherAction.Clipboard.action, clipboard.intent?.action)
        assertEquals(ComponentName(context, LauncherShortcutActivity::class.java), clipboard.intent?.component)
        LauncherShortcuts.publish(context)
        LauncherShortcuts.publish(context)
        assertEquals(setOf("send_file", "touchpad"), manager.dynamicShortcuts.map { it.id }.toSet())
        manager.dynamicShortcuts.forEach {
            assertTrue(it.shortLabel.toString().contains("niedostępne", ignoreCase = true))
            assertEquals(ComponentName(context, LauncherShortcutActivity::class.java), it.intent?.component)
            assertEquals(ComponentName(context, MainActivity::class.java), it.activity)
        }
    }

    @Test fun unavailableActionsOpenTheirOwnScreenAndCloseWithoutHome() {
        for (action in listOf(LauncherAction.File, LauncherAction.Touchpad)) {
            ActivityScenario.launch<LauncherShortcutActivity>(LauncherShortcuts.intent(context, action)).use {
                compose.onNodeWithText(action.title).assertIsDisplayed()
                compose.onNodeWithTag("shortcutUnavailable").assertIsDisplayed()
                compose.onNodeWithTag("computerCard").assertDoesNotExist()
                compose.onNodeWithText("Zamknij").performClick()
            }
        }
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
                        compose.onAllNodesWithText("Schowek niedostępny — sprawdź połączenie z komputerem").fetchSemanticsNodes().isNotEmpty()
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
