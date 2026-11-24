package io.github.prejas3.windconnect

import android.app.Instrumentation
import android.content.IntentFilter
import android.graphics.Bitmap
import android.os.ParcelFileDescriptor
import android.provider.Settings
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.dp
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.ui.WindApp
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import kotlinx.coroutines.runBlocking
import java.io.ByteArrayOutputStream

/**
 * WN-179: Google Play's prominent disclosure. Android's notification access screen (for notifications, WN-42, and the
 * player, WN-46) is reached from the app only through [ListenerDisclosure]: "Grant access" and turning a switch on
 * without access show it first, "Not now" opens nothing and leaves the switch off, "Continue" sends the system intent.
 * The system screen itself is caught by an ActivityMonitor and never opens. Needs notification access not granted
 * (a fresh install). Screenshots of both dialogs in light and dark go to /data/local/tmp/wn179 (adb pull).
 */
class ListenerDisclosureTest {
    @get:Rule val compose = createAndroidComposeRule<androidx.activity.ComponentActivity>()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val connections get() = (context.applicationContext as WindApplication).connections
    private lateinit var monitor: Instrumentation.ActivityMonitor
    private lateinit var core: io.github.prejas3.windconnect.core.WindCore

    @Before fun setUp() {
        val filter = IntentFilter().apply {
            addAction(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)
            addAction(Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS)
        }
        monitor = instrumentation.addMonitor(filter, null, true)
        core = runBlocking { connections.acquire() }
        allow(false)
    }

    @After fun tearDown() {
        instrumentation.removeMonitor(monitor)
        allow(false)
        runBlocking { connections.release(core) }
        context.getSharedPreferences("appearance", 0).edit().remove("theme").remove("dynamic").commit()
    }

    /** Both settings, applied (the owner sets them asynchronously). */
    private fun allow(on: Boolean) {
        connections.setNotifyAllowed(on); connections.setMediaAllowed(on)
        compose.waitUntil(5_000) { connections.notifyAllowed.value == on && connections.mediaAllowed.value == on }
    }

    private fun theme(mode: String) {
        context.getSharedPreferences("appearance", 0).edit().putString("theme", mode).putBoolean("dynamic", false).commit()
    }

    private fun show() {
        compose.setContent {
            Box(Modifier.width(412.dp).fillMaxHeight()) {
                WindApp(forceContent = true, onSettings = {}) {
                    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp), verticalArrangement = Arrangement.spacedBy(18.dp)) {
                        NotifyControls()
                        MediaControls()
                    }
                }
            }
        }
        compose.waitForIdle()
    }

    private fun dialogs() = compose.onAllNodesWithTag("listenerDisclosure", useUnmergedTree = true).fetchSemanticsNodes().size

    private fun grantShowsTheDisclosureFirst(grantTag: String, title: Int) {
        assertFalse("needs notification access not granted", connections.notifyAccess.value && connections.mediaAccess.value)
        val before = monitor.hits
        compose.onNodeWithTag(grantTag).performScrollTo().performClick()
        compose.waitForIdle()
        assertEquals("the disclosure, not the system screen", 1, dialogs())
        assertEquals(before, monitor.hits)
        compose.onNodeWithText(context.getString(title)).assertExists()
        compose.onNodeWithTag("disclosureNotNow").performClick()
        compose.waitForIdle()
        assertEquals(0, dialogs())
        assertEquals("Not now opens nothing", before, monitor.hits)
        compose.onNodeWithTag(grantTag).performScrollTo().performClick()
        compose.onNodeWithTag("disclosureContinue").performClick()
        compose.waitForIdle()
        assertEquals(0, dialogs())
        compose.waitUntil(5_000) { monitor.hits > before }
        assertEquals("Continue sends the system intent once", before + 1, monitor.hits)
    }

    @Test fun notificationsGrantAccessShowsTheDisclosureFirst() {
        show()
        grantShowsTheDisclosureFirst("notifyAccessSettings", R.string.notify_disclosure_title)
    }

    @Test fun playerGrantAccessShowsTheDisclosureFirst() {
        show()
        grantShowsTheDisclosureFirst("mediaAccessSettings", R.string.media_disclosure_title)
    }

    /** Turning a switch on without access: the disclosure; "Not now" leaves the setting off, "Continue" turns it on. */
    private fun switchOnWithoutAccess(switchTag: String, allowed: () -> Boolean) {
        show()
        val before = monitor.hits
        compose.onNodeWithTag(switchTag, useUnmergedTree = true).performScrollTo().performClick()
        compose.waitForIdle()
        assertEquals(1, dialogs())
        compose.onNodeWithTag("disclosureNotNow").performClick()
        compose.waitForIdle()
        assertFalse("Not now leaves the switch off", allowed())
        assertEquals(before, monitor.hits)
        compose.onNodeWithTag(switchTag, useUnmergedTree = true).performScrollTo().performClick()
        compose.onNodeWithTag("disclosureContinue").performClick()
        compose.waitUntil(5_000) { allowed() && monitor.hits > before }
    }

    @Test fun notificationsSwitchOnWithoutAccessShowsTheDisclosure() =
        switchOnWithoutAccess("notifyAllowed") { connections.notifyAllowed.value }

    @Test fun playerSwitchOnWithoutAccessShowsTheDisclosure() =
        switchOnWithoutAccess("mediaAllowed") { connections.mediaAllowed.value }

    private fun screenshots(mode: String) {
        theme(mode)
        show()
        for ((grant, name) in listOf("notifyAccessSettings" to "notifications", "mediaAccessSettings" to "player")) {
            compose.onNodeWithTag(grant).performScrollTo().performClick()
            compose.waitForIdle()
            save("$name-${mode.lowercase()}", compose.onNodeWithTag("listenerDisclosure", useUnmergedTree = true).captureToImage().asAndroidBitmap())
            compose.onNodeWithTag("disclosureNotNow").performClick()
            compose.waitForIdle()
        }
    }

    @Test fun screenshotsLight() = screenshots("Light")

    @Test fun screenshotsDark() = screenshots("Dark")

    private fun save(name: String, bitmap: Bitmap) {
        shell("mkdir -p /data/local/tmp/wn179")
        val bytes = ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
        val pipes = instrumentation.uiAutomation.executeShellCommandRw("dd of=/data/local/tmp/wn179/$name.png")
        ParcelFileDescriptor.AutoCloseOutputStream(pipes[1]).use { it.write(bytes) }
        ParcelFileDescriptor.AutoCloseInputStream(pipes[0]).use { it.readBytes() }
    }

    private fun shell(command: String): String = ParcelFileDescriptor.AutoCloseInputStream(
        instrumentation.uiAutomation.executeShellCommand(command)).use { String(it.readBytes()) }
}
