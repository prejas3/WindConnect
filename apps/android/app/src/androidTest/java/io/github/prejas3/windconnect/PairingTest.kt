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
    private fun openScanner() {
        compose.waitUntil(15_000) { compose.onAllNodesWithTag("pairingStatus").fetchSemanticsNodes().any { node ->
            node.config[androidx.compose.ui.semantics.SemanticsProperties.Text].none { it.text == "Przygotowywanie połączenia…" }
        } }
        if (compose.onAllNodesWithTag("scanQr").fetchSemanticsNodes().isEmpty()) compose.onNodeWithTag("openSettings").performClick()
        compose.waitUntil(15_000) { !compose.onNodeWithTag("scanQr").fetchSemanticsNode().config.contains(androidx.compose.ui.semantics.SemanticsProperties.Disabled) }
        compose.onNodeWithTag("scanQr").performScrollTo().performClick()
    }

    @Test fun settingsReturnsToHomeWithoutInventingAConnection() {
        compose.onNodeWithTag("openSettings").performClick()
        compose.onNodeWithTag("backgroundStatus").assertExists()
        compose.onNodeWithTag("closeSettings").performClick()
        compose.onNodeWithTag("computerCard").assertExists()
        compose.onNodeWithTag("recentTransfers").assertExists()
    }

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
        compose.onNodeWithTag("computerCard").assertExists()
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
        openScanner()
        compose.onNodeWithText("Zeskanuj kod QR").assertExists()
        compose.onNodeWithTag("cancelPairing").performScrollTo().performClick()
        compose.onNodeWithTag("computerCard").assertExists()
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

    @Test fun qrConnectsRealDesktop() {
        assumeTrue("requires real desktop QR image fixture", InstrumentationRegistry.getArguments().getString("desktopQr") == "true")
        val background = InstrumentationRegistry.getArguments().getString("backgroundService") == "true"
        val context = compose.activity.applicationContext
        if (InstrumentationRegistry.getArguments().getString("clipboardProof") == "true") {
            InstrumentationRegistry.getInstrumentation().runOnMainSync {
                context.getSystemService(android.content.ClipboardManager::class.java).setPrimaryClip(android.content.ClipData.newPlainText("WN35 old clip", "WN35-STALE-BEFORE-CONNECT"))
            }
        }
        if (background) assertFalse("app cannot acquire CPU wake locks", context.packageManager.getPackageInfo(context.packageName, android.content.pm.PackageManager.GET_PERMISSIONS).requestedPermissions?.contains(android.Manifest.permission.WAKE_LOCK) == true)
        if (android.os.Build.VERSION.SDK_INT >= 33) InstrumentationRegistry.getInstrumentation().uiAutomation.grantRuntimePermission(context.packageName, android.Manifest.permission.POST_NOTIFICATIONS)
        val file = File(compose.activity.cacheDir, "desktop-qr.png")
        val bitmap = android.graphics.BitmapFactory.decodeFile(file.absolutePath)
        val pixels = IntArray(bitmap.width * bitmap.height)
        bitmap.getPixels(pixels, 0, bitmap.width, 0, 0, bitmap.width, bitmap.height)
        val luminance = ByteArray(pixels.size) { ((pixels[it] shr 16) and 255).toByte() }
        val link = decodeQr(luminance, bitmap.width, bitmap.height) ?: throw AssertionError("PC canvas did not decode as QR")
        bitmap.recycle()
        val start = android.os.SystemClock.elapsedRealtime()
        deliver(link)
        waitFor("confirmPairing")
        compose.waitUntil(5000) { !compose.onNodeWithTag("confirmPairing").fetchSemanticsNode().config.contains(androidx.compose.ui.semantics.SemanticsProperties.Disabled) }
        compose.onNodeWithTag("confirmPairing").performScrollTo().performClick()
        compose.waitUntil(9000) { compose.onAllNodesWithTag("pairedConnectionStatus").fetchSemanticsNodes().any { it.config[androidx.compose.ui.semantics.SemanticsProperties.Text].any { text -> text.text == "Połączono z komputerem" } } }
        val elapsed = android.os.SystemClock.elapsedRealtime() - start
        assertTrue("decoded QR to connected session must stay below 10s: $elapsed", elapsed < 10000)
        InstrumentationRegistry.getInstrumentation().sendStatus(0, android.os.Bundle().apply { putString("qrToConnectedMs", elapsed.toString()) })
        if (InstrumentationRegistry.getArguments().getString("clipboardProof") == "true") {
            compose.onNodeWithTag("pairingDone").performScrollTo().performClick()
            ClipboardProof.run(compose)
            return
        }
        if (InstrumentationRegistry.getArguments().getString("homeScreenProof") == "true") {
            compose.onNodeWithTag("pairingDone").performScrollTo().performClick()
            compose.onNodeWithTag("computerName").assertTextEquals("Test PC")
            compose.onNodeWithTag("computerLatency").assertTextContains("ms", substring = true)
            for (tag in listOf("homeSend", "homeTouchpad", "homeLock", "homeLink")) compose.onNodeWithTag(tag).assertIsNotEnabled()
            compose.activityRule.scenario.recreate()
            compose.onNodeWithTag("computerName").assertTextEquals("Test PC")
            val instrumentation = InstrumentationRegistry.getInstrumentation()
            for (theme in listOf("Light", "Dark")) {
                compose.onNodeWithTag("previewToggle").performClick()
                if (android.os.Build.VERSION.SDK_INT >= 31) {
                    val source = compose.onNodeWithTag("paletteSource").fetchSemanticsNode().config[androidx.compose.ui.semantics.SemanticsProperties.Text].first().text
                    if (source.contains("systemowy")) compose.onNodeWithTag("dynamicColor").performClick()
                }
                compose.onNodeWithTag("theme-$theme").performScrollTo().performClick()
                compose.onNodeWithTag("previewToggle").performClick()
                compose.onNodeWithTag("computerName").assertTextEquals("Test PC")
                compose.waitForIdle()
                val image = instrumentation.uiAutomation.takeScreenshot()
                File(context.cacheDir, "wn31-$theme.png").outputStream().use { image.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
                image.recycle()
            }
            compose.onNodeWithTag("openSettings").performClick()
            compose.onNodeWithTag("backgroundStatus").assertExists()
            compose.onNodeWithTag("closeSettings").performClick()
            compose.onNodeWithTag("computerName").assertTextEquals("Test PC")
            instrumentation.sendStatus(0, android.os.Bundle().apply { putString("homeScreenProof", "true") })
            compose.activityRule.scenario.close()
        }
        if (background) {
            val manager = context.getSystemService(android.app.NotificationManager::class.java)
            val deadline = android.os.SystemClock.elapsedRealtime() + 5000
            while (manager.activeNotifications.none { it.id == ConnectionService.ID && it.notification.extras.getString(android.app.Notification.EXTRA_TEXT).orEmpty().startsWith("Połączono") } && android.os.SystemClock.elapsedRealtime() < deadline) Thread.sleep(50)
            fun notification() = manager.activeNotifications.first { it.id == ConnectionService.ID }.notification
            assertTrue(notification().extras.getString(android.app.Notification.EXTRA_TEXT).orEmpty().startsWith("Połączono"))
            assertEquals(2, notification().actions.size)
            compose.activityRule.scenario.close()
            fun shell(command: String): String = InstrumentationRegistry.getInstrumentation().uiAutomation.executeShellCommand(command).use { descriptor ->
                android.os.ParcelFileDescriptor.AutoCloseInputStream(descriptor).bufferedReader().use { it.readText() }
            }
            // CCS/1 LIMIT:test-only emulator power changes are restored in finally; physical overnight validation remains separate.
            assertEquals("1", shell("getprop ro.kernel.qemu").trim())
            val wasExempt = context.getSystemService(android.os.PowerManager::class.java).isIgnoringBatteryOptimizations(context.packageName)
            val exempt = InstrumentationRegistry.getArguments().getString("batteryExempt") != "false"
            val seconds = (InstrumentationRegistry.getArguments().getString("soakSeconds")?.toIntOrNull() ?: 60).coerceIn(60, 1800)
            var connectedChecks = 0
            try {
                shell("cmd deviceidle whitelist ${if (exempt) "+" else "-"}${context.packageName}")
                assertEquals(exempt, context.getSystemService(android.os.PowerManager::class.java).isIgnoringBatteryOptimizations(context.packageName))
                shell("dumpsys battery unplug")
                shell("input keyevent 223")
                val result = shell("dumpsys deviceidle force-idle")
                assertTrue(result, result.contains("idle", ignoreCase = true))
                repeat(seconds / 5) { index ->
                    Thread.sleep(5000)
                    val connected = notification().extras.getString(android.app.Notification.EXTRA_TEXT).orEmpty().startsWith("Połączono")
                    if (connected) connectedChecks++
                    if (exempt) assertTrue("session remains connected without Activity in Doze", connected)
                    if ((index + 1) % 12 == 0) {
                        val activeLocks = shell("dumpsys power").substringAfter("Wake Locks:").substringBefore("Suspend Blockers:")
                        assertFalse("no application wake lock during Doze", activeLocks.contains("WindConnect:session"))
                        InstrumentationRegistry.getInstrumentation().sendStatus(0, android.os.Bundle().apply {
                            putString("soakProgress", "${(index + 1) * 5}/$seconds seconds, exempt=$exempt, connectedChecks=$connectedChecks, wakeLock=false")
                        })
                    }
                }
                InstrumentationRegistry.getInstrumentation().sendStatus(0, android.os.Bundle().apply { putString("backgroundDozeComplete", "true") })
                shell("dumpsys deviceidle unforce")
                shell("dumpsys battery reset")
                shell("input keyevent 224")
                // CCS/1 RULE:without exemption Doze can block networking; waking must recover without a permanent CPU lock.
                val recovered = android.os.SystemClock.elapsedRealtime() + 75000
                while (!notification().extras.getString(android.app.Notification.EXTRA_TEXT).orEmpty().startsWith("Połączono") && android.os.SystemClock.elapsedRealtime() < recovered) Thread.sleep(200)
                assertTrue("connection recovers on wake", notification().extras.getString(android.app.Notification.EXTRA_TEXT).orEmpty().startsWith("Połączono"))
                val stop = notification().actions.first { it.title.toString() == "Zatrzymaj" }
                stop.actionIntent.send()
                val stopped = android.os.SystemClock.elapsedRealtime() + 5000
                while (manager.activeNotifications.any { it.id == ConnectionService.ID } && android.os.SystemClock.elapsedRealtime() < stopped) Thread.sleep(50)
                assertFalse(ConnectionService.enabled(context))
                assertTrue(manager.activeNotifications.none { it.id == ConnectionService.ID })
                val locks = shell("dumpsys power").substringAfter("Wake Locks:").substringBefore("Suspend Blockers:")
                assertFalse("notification stop releases active wake lock", locks.contains("WindConnect:session"))
                ConnectionBootReceiver().onReceive(context, Intent(Intent.ACTION_BOOT_COMPLETED))
                Thread.sleep(500)
                assertTrue("explicit stop survives boot", manager.activeNotifications.none { it.id == ConnectionService.ID })
                InstrumentationRegistry.getInstrumentation().sendStatus(0, android.os.Bundle().apply { putString("backgroundDozeSeconds", seconds.toString()); putString("batteryExempt", exempt.toString()); putString("connectedDozeChecks", connectedChecks.toString()); putString("wakeLock", "false"); putString("stopReleasesService", "true") })
                if (InstrumentationRegistry.getArguments().getString("backgroundBoot") == "true") {
                    assertTrue(ConnectionService.start(context, enable = true))
                    val restored = android.os.SystemClock.elapsedRealtime() + 15000
                    while (manager.activeNotifications.none { it.id == ConnectionService.ID && it.notification.extras.getString(android.app.Notification.EXTRA_TEXT).orEmpty().startsWith("Połączono") } && android.os.SystemClock.elapsedRealtime() < restored) Thread.sleep(100)
                    assertTrue(notification().extras.getString(android.app.Notification.EXTRA_TEXT).orEmpty().startsWith("Połączono"))
                }
            } finally {
                shell("dumpsys deviceidle unforce")
                shell("dumpsys battery reset")
                shell("input keyevent 224")
                shell("cmd deviceidle whitelist ${if (wasExempt) "+" else "-"}${context.packageName}")
            }
        }
        Thread.sleep(2000) // CCS/1 PURPOSE:bounded evidence window for host to capture the actual connected desktop UI.
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
            openScanner()
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
