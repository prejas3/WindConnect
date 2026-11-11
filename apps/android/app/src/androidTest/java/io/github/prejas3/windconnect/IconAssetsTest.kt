package io.github.prejas3.windconnect

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.drawable.AdaptiveIconDrawable
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Test
import java.io.File
import android.app.NotificationManager
import android.os.SystemClock
import androidx.test.core.app.ActivityScenario

/** CCS/1 CHECK:actual packaged Android drawables, safe zone/alpha and monochrome; no launcher/One UI claim. */
class IconAssetsTest {
    private val context get() = InstrumentationRegistry.getInstrumentation().targetContext
    private fun bitmap(id: Int, size: Int): Bitmap {
        val drawable = context.getDrawable(id)!!
        return Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888).also {
            drawable.setBounds(0, 0, size, size); drawable.draw(Canvas(it))
        }
    }
    @Test fun adaptiveMonochromeAndNotificationKeepTheOrbitaSilhouette() {
        val foreground = bitmap(R.drawable.ic_launcher_foreground, 108)
        val mono = bitmap(R.drawable.ic_launcher_monochrome, 108)
        assertTrue(context.getDrawable(R.mipmap.ic_launcher) is AdaptiveIconDrawable)
        assertTrue(context.getDrawable(R.mipmap.ic_launcher_round) is AdaptiveIconDrawable)
        var visible = 0
        for (y in 0 until 108) for (x in 0 until 108) {
            val pixel = foreground.getPixel(x, y)
            assertEquals(pixel, mono.getPixel(x, y))
            if (Color.alpha(pixel) > 16) {
                visible++
                assertTrue("adaptive mark outside 66dp safe circle at $x,$y", Math.hypot(x + .5 - 54, y + .5 - 54) <= 33.5)
                assertEquals(Color.WHITE, pixel or (0xff shl 24))
            }
        }
        assertTrue(visible > 500)
        val notification = bitmap(R.drawable.ic_connection, 96)
        assertEquals(0, Color.alpha(notification.getPixel(0, 0)))
        var antialiased = false
        for (y in 0 until 96) for (x in 0 until 96) {
            val pixel = notification.getPixel(x, y)
            if (Color.alpha(pixel) > 0) assertEquals(Color.WHITE, pixel or (0xff shl 24))
            if (Color.alpha(pixel) in 1..254) antialiased = true
        }
        assertTrue(antialiased)
        val sheet = Bitmap.createBitmap(900, 420, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(sheet)
        val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { textSize = 22f; color = Color.rgb(21,40,47) }
        canvas.drawColor(Color.rgb(232,239,239))
        canvas.drawText("Orbita / packaged Android resources", 28f, 38f, paint)
        val icons = listOf(R.mipmap.ic_launcher, R.mipmap.ic_launcher_round, R.drawable.ic_launcher_monochrome, R.drawable.ic_connection)
        val labels = listOf("Adaptive mask", "Round launcher", "Monochrome", "Notification")
        for ((index, id) in icons.withIndex()) {
            val x = 28 + index * 218
            canvas.drawRect(x.toFloat(), 75f, x + 192f, 267f, Paint().apply { color = Color.rgb(22,36,43) })
            val drawable = context.getDrawable(id)!!
            drawable.setBounds(x, 75, x + 192, 267); drawable.draw(canvas)
            canvas.drawText(labels[index], x.toFloat(), 310f, paint)
        }
        File(context.cacheDir, "wn93-icons.png").outputStream().use { sheet.compress(Bitmap.CompressFormat.PNG, 100, it) }
        foreground.recycle(); mono.recycle(); notification.recycle(); sheet.recycle()
    }
    @Test fun persistentNotificationUsesThePackagedWhiteOrbita() {
        org.junit.Assume.assumeTrue("requires explicitly verified disposable emulator", InstrumentationRegistry.getArguments().getString("iconFixture") == "true")
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        if (android.os.Build.VERSION.SDK_INT >= 33) instrumentation.uiAutomation.grantRuntimePermission(context.packageName, android.Manifest.permission.POST_NOTIFICATIONS)
        val wasEnabled = ConnectionService.enabled(context)
        // CCS/1 CHECK:synthetic offline pairing keeps the production service alive; no authenticated-session claim. Restore the fixture on exit.
        val paired = File(context.noBackupFilesDir, "paired.txt")
        val previous = if (paired.exists()) paired.readBytes() else null
        val key = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a" // RFC8032 public test vector, no network peer.
        paired.writeText("\n$key 1 Icon%20fixture%20offline\n")
        var scenario: ActivityScenario<MainActivity>? = null
        try {
            kotlinx.coroutines.runBlocking {
                val owner = (context.applicationContext as WindApplication).connections
                val core = owner.acquire()
                try { assertEquals("Icon fixture offline", core.pairedDevices().single().name) }
                finally { owner.release(core) }
            }
            val opened = ActivityScenario.launch(MainActivity::class.java)
            scenario = opened
            opened.onActivity { assertTrue(ConnectionService.start(it, true)) }
            val manager = context.getSystemService(NotificationManager::class.java)
            // CCS/1 WHY:Android 12+ may defer foreground notification visibility for 10s; wait for the posted record, not a fixed sleep.
            val until = SystemClock.uptimeMillis() + 20_000
            while (manager.activeNotifications.none { it.id == ConnectionService.ID } && SystemClock.uptimeMillis() < until) Thread.sleep(25)
            val notification = manager.activeNotifications.singleOrNull { it.id == ConnectionService.ID }?.notification
                ?: throw AssertionError("foreground notification not posted; enabled=${ConnectionService.enabled(context)}; allowed=${manager.areNotificationsEnabled()}; serviceError=${ConnectionService.preferences(context).getString("error", null)}")
            assertEquals(R.drawable.ic_connection, notification.smallIcon.resId)
            assertTrue(instrumentation.uiAutomation.performGlobalAction(android.accessibilityservice.AccessibilityService.GLOBAL_ACTION_NOTIFICATIONS))
            val automation = instrumentation.uiAutomation
            automation.serviceInfo = automation.serviceInfo.apply {
                flags = flags or android.accessibilityservice.AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS
            }
            val shadeUntil = SystemClock.uptimeMillis() + 5_000
            fun titleVisible(): Boolean {
                val pending = java.util.ArrayDeque<android.view.accessibility.AccessibilityNodeInfo>()
                automation.windows.forEach { window -> window.root?.let { pending.add(it) } }
                automation.rootInActiveWindow?.let { pending.add(it) }
                while (pending.isNotEmpty()) {
                    val node = pending.removeFirst()
                    if (node.text?.toString()?.contains("Icon fixture offline") == true) return true
                    repeat(node.childCount) { node.getChild(it)?.let { child -> pending.add(child) } }
                }
                return false
            }
            while (!titleVisible() && SystemClock.uptimeMillis() < shadeUntil) Thread.sleep(25)
            automation.waitForIdle(200, 5_000)
            val image = instrumentation.uiAutomation.takeScreenshot()
            File(context.cacheDir, "wn93-notification.png").outputStream().use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
            image.recycle()
            assertTrue("notification title visible in SystemUI; screenshot saved", titleVisible())
        } finally {
            instrumentation.uiAutomation.performGlobalAction(android.accessibilityservice.AccessibilityService.GLOBAL_ACTION_BACK)
            if (!wasEnabled) ConnectionService.stop(context)
            try { scenario?.close() } finally { if (previous == null) paired.delete() else paired.writeBytes(previous) }
        }
    }
}
