package io.github.prejas3.windconnect

import android.content.Intent
import android.net.Uri
import android.util.Base64
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.Assume.assumeTrue
import java.io.File

class PairingTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private val secret = "EREREREREREREREREREREQ"
    private fun link(name: String = "Test%20PC"): String {
        val key = Base64.encodeToString(ByteArray(32) { if (it == 0) 0x58.toByte() else 0x66.toByte() }, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)
        return "windconnect:pair?v=1&k=$key&s=$secret&a=127.0.0.1%3A9&n=$name"
    }
    private fun deliver(value: String, explicit: Boolean = false) {
        val intent = Intent(Intent.ACTION_VIEW, Uri.parse(value)).addCategory(Intent.CATEGORY_BROWSABLE).setPackage(compose.activity.packageName)
        if (explicit) intent.setClass(compose.activity, MainActivity::class.java)
        assertNotNull("Android resolves the camera's VIEW intent", intent.resolveActivity(compose.activity.packageManager))
        compose.runOnUiThread { compose.activity.startActivity(intent) }
    }
    private fun waitFor(tag: String) { compose.waitUntil(10_000) { compose.onAllNodesWithTag(tag).fetchSemanticsNodes().isNotEmpty() } }
    private fun trust(): String = File(compose.activity.noBackupFilesDir, "paired.txt").let { if (it.exists()) it.readText() else "" }

    @Test fun externalViewRequiresConfirmationAndCancellationDoesNotTrust() {
        val before = trust()
        compose.onNodeWithTag("previewToggle").performClick()
        deliver(link())
        waitFor("confirmPairing")
        compose.onNodeWithTag("pairingPeer").assertTextEquals("Test PC")
        assertNull(compose.activity.intent.data)
        assertTrue(compose.onAllNodesWithText(secret, substring = true).fetchSemanticsNodes().isEmpty())
        assertEquals(before, trust())
        compose.activityRule.scenario.recreate()
        waitFor("confirmPairing")
        compose.onNodeWithTag("cancelPairing").performScrollTo().performClick()
        compose.onNodeWithTag("scanQr").assertExists()
        assertEquals(before, trust())
        deliver(link("Drugi%20PC"))
        waitFor("confirmPairing")
        compose.onNodeWithTag("pairingPeer").assertTextEquals("Drugi PC")
    }

    @Test fun invalidOrUnrelatedIntentShowsSafeError() {
        val before = trust()
        for (value in listOf("https://example.org/", "windconnect:other", link() + "&v=1", "windconnect:pair?" + "x".repeat(1025))) {
            deliver(value, explicit = true)
            waitFor("pairingError")
            assertTrue(compose.onAllNodesWithText(secret, substring = true).fetchSemanticsNodes().isEmpty())
            assertEquals(before, trust())
            compose.onNodeWithTag("cancelPairing").performScrollTo().performClick()
        }
    }

    @Test fun qrPixelsDecodeOfflineInEachOrientation() {
        val text = link()
        val matrix = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, 480, 480)
        var pixels = ByteArray(480 * 480) { if (matrix[it % 480, it / 480]) 0 else 255.toByte() }
        repeat(4) {
            assertEquals(text, decodeQr(pixels, 480, 480))
            val rotated = ByteArray(pixels.size)
            for (y in 0 until 480) for (x in 0 until 480) rotated[x * 480 + 479 - y] = pixels[y * 480 + x]
            pixels = rotated
        }
        assertNull(decodeQr(ByteArray(480 * 480) { 255.toByte() }, 480, 480))
    }

    @Test fun scannerEntryAndCancelRemainAvailable() {
        compose.waitUntil(15_000) { !compose.onNodeWithTag("scanQr").fetchSemanticsNode().config.contains(androidx.compose.ui.semantics.SemanticsProperties.Disabled) }
        compose.onNodeWithTag("scanQr").performClick()
        compose.onNodeWithText("Zeskanuj kod QR").assertExists()
        compose.onNodeWithTag("cancelPairing").performScrollTo().performClick()
        compose.onNodeWithTag("scanQr").assertExists()
    }

    @Test fun explicitConfirmationPairsWithRealPc() {
        val value = InstrumentationRegistry.getArguments().getString("pairingLink")
        assumeTrue("requires local QR pairing fixture", value != null)
        deliver(value!!)
        waitFor("confirmPairing")
        compose.waitUntil(15_000) { !compose.onNodeWithTag("confirmPairing").fetchSemanticsNode().config.contains(androidx.compose.ui.semantics.SemanticsProperties.Disabled) }
        compose.onNodeWithTag("confirmPairing").performScrollTo().performClick()
        compose.waitUntil(40_000) { compose.onAllNodesWithTag("pairingSuccess").fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithText("Test PC").assertExists()
        assertTrue(trust().isNotBlank())
    }

    @Test fun cameraWorkerStopsAfterLeavingScanner() {
        InstrumentationRegistry.getInstrumentation().uiAutomation.grantRuntimePermission(compose.activity.packageName, android.Manifest.permission.CAMERA)
        compose.activityRule.scenario.recreate()
        val manager = compose.activity.getSystemService(android.hardware.camera2.CameraManager::class.java)
        val cameraId = manager.cameraIdList.firstOrNull { manager.getCameraCharacteristics(it).get(android.hardware.camera2.CameraCharacteristics.LENS_FACING) == android.hardware.camera2.CameraCharacteristics.LENS_FACING_BACK } ?: manager.cameraIdList.first()
        val available = java.util.concurrent.atomic.AtomicBoolean(true)
        val callback = object : android.hardware.camera2.CameraManager.AvailabilityCallback() {
            override fun onCameraAvailable(id: String) { if (id == cameraId) available.set(true) }
            override fun onCameraUnavailable(id: String) { if (id == cameraId) available.set(false) }
        }
        manager.registerAvailabilityCallback(callback, android.os.Handler(android.os.Looper.getMainLooper()))
        try {
            compose.waitUntil(15_000) { !compose.onNodeWithTag("scanQr").fetchSemanticsNode().config.contains(androidx.compose.ui.semantics.SemanticsProperties.Disabled) }
            compose.onNodeWithTag("scanQr").performClick()
            compose.onNodeWithTag("cameraPreview").assertExists()
            try {
                compose.waitUntil(5_000) { Thread.getAllStackTraces().keys.any { it.name.startsWith("WindConnect QR") && it.isAlive } }
            } catch (error: Exception) {
                throw AssertionError("QR camera worker absent; threads=${Thread.getAllStackTraces().keys.map { it.name }}; cameraError=${compose.onAllNodesWithTag("pairingError").fetchSemanticsNodes().size}", error)
            }
            compose.waitUntil(5_000) { !available.get() }
            compose.activityRule.scenario.moveToState(androidx.lifecycle.Lifecycle.State.CREATED)
            compose.waitUntil(5_000) { available.get() }
            compose.activityRule.scenario.moveToState(androidx.lifecycle.Lifecycle.State.RESUMED)
            compose.waitUntil(5_000) { !available.get() }
            compose.onNodeWithTag("cancelPairing").performScrollTo().performClick()
            compose.waitUntil(5_000) { available.get() }
            compose.waitUntil(5_000) { Thread.getAllStackTraces().keys.none { it.name.startsWith("WindConnect QR") && it.isAlive } }
        } finally { manager.unregisterAvailabilityCallback(callback) }
    }
}
