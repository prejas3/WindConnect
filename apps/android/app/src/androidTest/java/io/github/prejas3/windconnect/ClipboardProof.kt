package io.github.prejas3.windconnect

import android.content.ClipData
import android.content.ClipboardManager
import android.os.Bundle
import android.os.PersistableBundle
import android.os.SystemClock
import androidx.compose.ui.test.junit4.AndroidComposeTestRule
import androidx.compose.ui.test.*
import androidx.lifecycle.Lifecycle
import androidx.test.ext.junit.rules.ActivityScenarioRule
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.core.ConnectionState
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import java.io.File

/** CCS/1 PURPOSE:real OS->QUIC->OS applied ack; synthetic text and images (WN-36) only, no clipboard read in background. */
object ClipboardProof {
    fun run(compose: AndroidComposeTestRule<ActivityScenarioRule<MainActivity>, MainActivity>) = runBlocking {
        val activity = compose.activity
        val application = activity.application as WindApplication
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val manager = activity.getSystemService(ClipboardManager::class.java)
        val core = application.connections.acquire()
        fun copy(text: String, sensitive: Boolean = false) {
            instrumentation.runOnMainSync {
                val clip = ClipData.newPlainText("WN35 test", text)
                if (sensitive) clip.description.extras = PersistableBundle().apply { putBoolean("android.content.extra.IS_SENSITIVE", true) }
                manager.setPrimaryClip(clip)
                if (activity.hasWindowFocus()) {
                    val applied = manager.primaryClip
                    assertEquals("OS fixture must retain copied value", text, applied?.getItemAt(0)?.text?.toString())
                    assertEquals("OS fixture must retain sensitivity", sensitive, applied?.description?.extras?.getBoolean("android.content.extra.IS_SENSITIVE", false) ?: false)
                }
            }
        }
        suspend fun awaitDelivered(before: Long) {
            try { withTimeout(5_000) { core.clipboardUi.first { it.delivered > before } } }
            catch (error: TimeoutCancellationException) {
                throw AssertionError("automatic delivery absent: ui=${core.clipboardUi.value}; focus=${activity.hasWindowFocus()}; timestamp=${manager.primaryClipDescription?.timestamp}; now=${System.currentTimeMillis()}; connection=${core.state.value}; manualProbe=${core.sendCurrentClipboard()}", error)
            }
        }
        try {
            withTimeout(10_000) { core.state.first { it is ConnectionState.Connected } }
            compose.waitUntil(5_000) { compose.activity.hasWindowFocus() }
            core.setClipboardEnabled(true)
            delay(200)
            assertEquals("pre-session clip must not be sent on focus", 0L, core.clipboardUi.value.delivered)
            compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
            copy("WN35-NEW-AFTER-CONNECT")
            delay(100)
            assertEquals("background clip awaits focus", 0L, core.clipboardUi.value.delivered)
            compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
            compose.waitForIdle()
            awaitDelivered(0L)
            val samples = mutableListOf<Long>()
            val runId = SystemClock.elapsedRealtimeNanos()
            repeat(100) { index ->
                val before = core.clipboardUi.value.delivered
                val start = SystemClock.elapsedRealtimeNanos()
                copy("WN35-ANDROID-BENCH-$runId-$index 👩‍💻 https://example.com/ż")
                awaitDelivered(before)
                samples += (SystemClock.elapsedRealtimeNanos() - start) / 1_000
            }
            instrumentation.sendStatus(0, Bundle().apply { putString("androidToPcP95Micros", samples.sorted()[94].toString()) })
            val beforePrivate = core.clipboardUi.value.delivered
            copy("WN35-PRIVATE-ANDROID", true)
            assertEquals("sensitive", core.sendCurrentClipboard())
            delay(150)
            assertEquals(beforePrivate, core.clipboardUi.value.delivered)
            core.setClipboardEnabled(false)
            copy("WN35-ANDROID-MANUAL")
            delay(100)
            assertEquals(beforePrivate, core.clipboardUi.value.delivered)
            assertEquals("applied", core.sendCurrentClipboard())
            core.setClipboardEnabled(true)

            val beforePc = core.clipboardUi.value.received
            val beforeControl = core.clipboardUi.value.delivered
            copy("WN35-CONTROL-PC-BENCH")
            awaitDelivered(beforeControl)
            compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
            withTimeout(30_000) { core.clipboardUi.first { it.received >= beforePc + 100 } }
            compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
            compose.waitForIdle()
            instrumentation.runOnMainSync {
                assertEquals("WN35-PC-BENCH-99", manager.primaryClip!!.getItemAt(0).text.toString())
                assertTrue(manager.primaryClip!!.description.extras!!.getBoolean("android.content.extra.IS_SENSITIVE"))
            }
            val receivedBeforePrivate = core.clipboardUi.value.received
            val deliveredBeforePrivate = core.clipboardUi.value.delivered
            copy("WN35-CONTROL-PC-PRIVATE")
            awaitDelivered(deliveredBeforePrivate)
            withTimeout(5_000) { core.clipboardUi.first { it.received > receivedBeforePrivate } }
            delay(100)
            assertEquals(receivedBeforePrivate + 1, core.clipboardUi.value.received)
            instrumentation.runOnMainSync { assertEquals("WN35-PC-PRIVATE-CHECK", manager.primaryClip!!.getItemAt(0).text.toString()) }

            // WN-36: PC screenshots (10 × 1080p, 3 × 4K) arrive in the background as text does; the PC times them.
            val receivedBeforeImages = core.clipboardUi.value.received
            val deliveredBeforeImages = core.clipboardUi.value.delivered
            copy("WN36-CONTROL-PC-IMAGE")
            awaitDelivered(deliveredBeforeImages)
            compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
            withTimeout(120_000) { core.clipboardUi.first { it.received >= receivedBeforeImages + 13 } }
            compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
            compose.waitForIdle()
            var imageUri: android.net.Uri? = null
            instrumentation.runOnMainSync {
                val clip = manager.primaryClip!!
                assertTrue("an image clip", clip.description.hasMimeType("image/png"))
                assertNull(clip.getItemAt(0).text)
                assertFalse("images are not marked sensitive (preview works)", clip.description.extras?.getBoolean("android.content.extra.IS_SENSITIVE", false) ?: false)
                imageUri = clip.getItemAt(0).uri
            }
            // A paste: the whole PNG through the URI; a preview: the thumbnail.
            val pasted = activity.contentResolver.openInputStream(imageUri!!)!!.use { it.readBytes() }
            val bounds = android.graphics.BitmapFactory.Options().apply { inJustDecodeBounds = true }
            android.graphics.BitmapFactory.decodeByteArray(pasted, 0, pasted.size, bounds)
            assertEquals(3840 to 2160, bounds.outWidth to bounds.outHeight)
            val preview = activity.contentResolver.loadThumbnail(imageUri!!, android.util.Size(96, 96), null)
            assertTrue(preview.width in 1..512)
            delay(300)
            assertEquals("our own received image is never sent back", deliveredBeforeImages + 1, core.clipboardUi.value.delivered)

            // Phone -> PC: an image another app put on the clipboard (MediaStore URI), sent when focused.
            val values = android.content.ContentValues().apply {
                put(android.provider.MediaStore.Images.Media.DISPLAY_NAME, "wn36-test-${SystemClock.elapsedRealtimeNanos()}.png")
                put(android.provider.MediaStore.Images.Media.MIME_TYPE, "image/png")
                put(android.provider.MediaStore.Images.Media.RELATIVE_PATH, "Pictures/WindConnectTest")
            }
            val stored = activity.contentResolver.insert(android.provider.MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values)!!
            try {
                activity.contentResolver.openOutputStream(stored)!!.use { out ->
                    val bitmap = android.graphics.Bitmap.createBitmap(800, 600, android.graphics.Bitmap.Config.ARGB_8888).apply { eraseColor(android.graphics.Color.rgb(20, 120, 110)) }
                    bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, out)
                }
                val beforePhoneImage = core.clipboardUi.value.delivered
                instrumentation.runOnMainSync { manager.setPrimaryClip(ClipData.newUri(activity.contentResolver, "WN36 image", stored)) }
                awaitDelivered(beforePhoneImage)
                assertEquals("Obraz wysłany na komputer", core.clipboardUi.value.status)
            } finally { activity.contentResolver.delete(stored, null, null) }

            compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
            copy("WN35-ANDROID-NOTIFICATION")
            delay(200)
            val beforeNotification = core.clipboardUi.value.delivered
            tapNotification(instrumentation, File(activity.cacheDir, "wn35-notification.png"))
            withTimeout(15_000) { core.clipboardUi.first { it.delivered > beforeNotification } }
            compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
            compose.waitForIdle()
            val count = core.clipboardUi.value.delivered
            compose.activityRule.scenario.recreate()
            compose.waitForIdle()
            delay(200)
            assertEquals("focus/recreation must not resend the same clip", count, core.clipboardUi.value.delivered)

            for (theme in listOf("Light", "Dark")) {
                compose.onNodeWithTag("previewToggle").performClick()
                if (android.os.Build.VERSION.SDK_INT >= 31) {
                    val source = compose.onNodeWithTag("paletteSource").fetchSemanticsNode().config[androidx.compose.ui.semantics.SemanticsProperties.Text].first().text
                    if (source.contains("systemowy")) compose.onNodeWithTag("dynamicColor").performClick()
                }
                compose.onNodeWithTag("theme-$theme").performScrollTo().performClick()
                compose.onNodeWithTag("previewToggle").performClick()
                compose.onNodeWithTag("clipboardCard").performScrollTo()
                compose.waitForIdle()
                val image = instrumentation.uiAutomation.takeScreenshot()
                File(activity.cacheDir, "wn35-$theme.png").outputStream().use { image.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
                image.recycle()
            }

            val sorted = samples.sorted()
            val report = org.json.JSONObject().apply {
                put("samplesMicros", org.json.JSONArray(samples))
                put("p95Micros", sorted[94]); put("maxMicros", sorted[99]); put("samples", samples.size)
                put("startsAt", "Android OS clipboard copy"); put("endsAt", "ack after native Windows clipboard write")
                put("pcToAndroidInBackground", true); put("sensitiveBlocked", true); put("notification", true); put("noFocusEcho", true)
            }
            File(activity.cacheDir, "wn35-android-benchmark.json").writeText(report.toString(2))
            instrumentation.sendStatus(0, Bundle().apply { putString("clipboardProof", "true") })
        } finally { application.connections.release(core) }
    }

    private fun tapNotification(instrumentation: android.app.Instrumentation, screenshot: File) {
        val automation = instrumentation.uiAutomation
        automation.serviceInfo = automation.serviceInfo.apply {
            flags = flags or android.accessibilityservice.AccessibilityServiceInfo.FLAG_REPORT_VIEW_IDS or android.accessibilityservice.AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS
        }
        assertTrue(automation.performGlobalAction(android.accessibilityservice.AccessibilityService.GLOBAL_ACTION_NOTIFICATIONS))
        fun root() = automation.rootInActiveWindow
        fun button(): android.view.accessibility.AccessibilityNodeInfo? {
            val pending = java.util.ArrayDeque<android.view.accessibility.AccessibilityNodeInfo>()
            automation.windows.forEach { window -> window.root?.let { pending.add(it) } }
            root()?.let { pending.add(it) }
            while (pending.isNotEmpty()) {
                val node = pending.removeFirst()
                if (node.text?.toString()?.contains("Wyślij schowek", ignoreCase = true) == true ||
                    node.contentDescription?.toString()?.contains("Wyślij schowek", ignoreCase = true) == true) return node
                repeat(node.childCount) { node.getChild(it)?.let { child -> pending.add(child) } }
            }
            return null
        }
        fun click(node: android.view.accessibility.AccessibilityNodeInfo) {
            var current: android.view.accessibility.AccessibilityNodeInfo? = node
            repeat(8) {
                val candidate = current ?: return@repeat
                if (candidate.isClickable) { assertTrue(candidate.performAction(android.view.accessibility.AccessibilityNodeInfo.ACTION_CLICK)); return }
                current = candidate.parent
            }
            throw AssertionError("notification node has no clickable ancestor")
        }
        val deadline = SystemClock.uptimeMillis() + 5_000
        var expanded = false
        while (button() == null && SystemClock.uptimeMillis() < deadline) {
            if (!expanded) {
                val pending = java.util.ArrayDeque<android.view.accessibility.AccessibilityNodeInfo>()
                root()?.let { pending.add(it) }
                while (pending.isNotEmpty()) {
                    val node = pending.removeFirst()
                    val description = node.contentDescription?.toString().orEmpty()
                    if (description.startsWith("Expand") || description.startsWith("Rozwiń")) { click(node); expanded = true; break }
                    repeat(node.childCount) { node.getChild(it)?.let { child -> pending.add(child) } }
                }
            }
            Thread.sleep(100)
        }
        val image = automation.takeScreenshot()
        screenshot.outputStream().use { image.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
        image.recycle()
        val action = button() ?: run {
            val pending = java.util.ArrayDeque<android.view.accessibility.AccessibilityNodeInfo>()
            root()?.let { pending.add(it) }
            val labels = mutableListOf<String>()
            while (pending.isNotEmpty() && labels.size < 300) {
                val node = pending.removeFirst()
                labels += "${node.text?.toString().orEmpty()}|${node.contentDescription?.toString().orEmpty()}|${node.viewIdResourceName.orEmpty()}"
                repeat(node.childCount) { node.getChild(it)?.let { child -> pending.add(child) } }
            }
            throw AssertionError("real clipboard notification action absent; expanded=$expanded; labels=$labels")
        }
        click(action)
    }
}
