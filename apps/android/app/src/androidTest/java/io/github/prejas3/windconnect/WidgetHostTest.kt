package io.github.prejas3.windconnect

import android.appwidget.*
import android.content.*
import android.content.res.Configuration
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.*
import android.view.View
import android.view.ViewGroup
import android.widget.*
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import androidx.compose.ui.graphics.toArgb
import io.github.prejas3.windconnect.ui.BrandLight
import io.github.prejas3.windconnect.ui.BrandDark
import org.junit.Assert.*
import org.junit.Test
import java.io.File
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

/** CCS/1 CHECK:real RemoteViews host on disposable emulator only; restore battery/prefs and delete host IDs. */
class WidgetHostTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private fun shell(command: String): String = ParcelFileDescriptor.AutoCloseInputStream(
        instrumentation.uiAutomation.executeShellCommand(command)).use { String(it.readBytes()) }
    private fun waitFor(message: String, timeout: Long = 30_000, condition: () -> Boolean) {
        val deadline = SystemClock.elapsedRealtime() + timeout
        while (!condition()) {
            assertTrue(message, SystemClock.elapsedRealtime() < deadline)
            Thread.sleep(50)
        }
    }
    private fun nodes(view: View): List<View> = listOf(view) +
        if (view is ViewGroup) (0 until view.childCount).flatMap { nodes(view.getChildAt(it)) } else emptyList()
    private fun texts(view: View) = nodes(view).filterIsInstance<TextView>()
    private fun textExists(view: View, text: String): Boolean {
        var exists = false
        instrumentation.runOnMainSync { exists = texts(view).any { it.text.toString().contains(text) } }
        return exists
    }
    private class CountingView(context: Context) : AppWidgetHostView(context) {
        val count = AtomicInteger()
        val events = LinkedBlockingQueue<Int>()
        override fun updateAppWidget(views: RemoteViews?) {
            super.updateAppWidget(views)
            events.offer(count.incrementAndGet())
        }
    }

    @Test fun nativeWidgetEventsActionsThemesAndTwoMinuteIdleBudget() {
        org.junit.Assume.assumeTrue("requires own disposable emulator fixture", InstrumentationRegistry.getArguments().getString("widgetFixture") == "true")
        assertEquals("1", shell("getprop ro.kernel.qemu").trim())
        val preferences = ConnectionService.preferences(context)
        val appearance = context.getSharedPreferences("appearance", Context.MODE_PRIVATE)
        val wasEnabled = preferences.getBoolean("enabled", false)
        val oldTheme = appearance.getString("theme", "System")
        val oldDynamic = appearance.getBoolean("dynamic", true)
        preferences.edit().putBoolean("enabled", false).commit()
        appearance.edit().putString("theme", "Light").putBoolean("dynamic", false).commit()
        shell("appwidget grantbind --package ${context.packageName} --user 0")
        shell("dumpsys battery set level 77")
        shell("dumpsys battery set status 3")
        val configuration = Configuration(context.resources.configuration).apply { fontScale = 1.5f }
        val hostContext = context.createConfigurationContext(configuration)
        val host = object : AppWidgetHost(context, 330033) {
            override fun onCreateView(context: Context, appWidgetId: Int, appWidget: AppWidgetProviderInfo) = CountingView(hostContext)
        }
        val manager = AppWidgetManager.getInstance(context)
        val id = host.allocateAppWidgetId()
        var scenario: ActivityScenario<MainActivity>? = null
        try {
            val component = ComponentName(context, WindWidgetReceiver::class.java)
            assertTrue("bind own provider", manager.bindAppWidgetIdIfAllowed(id, component))
            val info = manager.getAppWidgetInfo(id)
            assertNotNull(info)
            assertEquals(0, info.updatePeriodMillis)
            val options = Bundle().apply {
                putInt(AppWidgetManager.OPTION_APPWIDGET_MIN_WIDTH, 320)
                putInt(AppWidgetManager.OPTION_APPWIDGET_MAX_WIDTH, 320)
                putInt(AppWidgetManager.OPTION_APPWIDGET_MIN_HEIGHT, 220)
                putInt(AppWidgetManager.OPTION_APPWIDGET_MAX_HEIGHT, 220)
            }
            manager.updateAppWidgetOptions(id, options)
            val opened = ActivityScenario.launch(MainActivity::class.java)
            scenario = opened
            lateinit var view: CountingView
            opened.onActivity { activity ->
                host.startListening()
                view = host.createView(activity, id, info) as CountingView
                val density = activity.resources.displayMetrics.density
                activity.setContentView(FrameLayout(activity).apply {
                    addView(view, FrameLayout.LayoutParams((320*density).toInt(), (220*density).toInt(), android.view.Gravity.CENTER))
                })
            }
            waitFor("phone battery visible") { textExists(view, "Telefon: 77%") }
            waitFor("background-off label visible") { textExists(view, "Połączenie w tle wyłączone") }
            for (mode in listOf("Light", "Dark")) {
                appearance.edit().putString("theme", mode).commit()
                waitFor("theme model update") { WidgetUpdates.snapshot.value.data.theme == mode }
                val expectedInk = (if (mode == "Light") BrandLight else BrandDark).onSurface.toArgb()
                waitFor("theme RemoteViews applied and laid out") {
                    var ready = false
                    instrumentation.runOnMainSync {
                        ready = !view.isLayoutRequested && texts(view).any {
                            it.text.toString() == "Twój komputer" && it.currentTextColor == expectedInk && it.width > 0 && it.height > 0
                        }
                    }
                    ready
                }
                waitFor("all widget actions laid out") {
                    var ready = false
                    instrumentation.runOnMainSync { ready = texts(view).filter { it.text.toString() in listOf("Wyślij schowek", "Otwórz") }.size == 2 }
                    ready
                }
                var bounds = android.graphics.Rect()
                instrumentation.runOnMainSync {
                    texts(view).filter { it.text.toString() in listOf("Wyślij schowek", "Otwórz") }.forEach {
                        assertNotNull(it.layout)
                        for (line in 0 until it.layout.lineCount) assertEquals("action clipped at150%: ${it.text}", 0, it.layout.getEllipsisCount(line))
                        assertTrue("text taller than its view", it.layout.height <= it.height)
                    }
                    val location = IntArray(2)
                    view.getLocationOnScreen(location)
                    bounds = android.graphics.Rect(location[0], location[1], location[0] + view.width, location[1] + view.height)
                }
                instrumentation.waitForIdleSync()
                val screen = instrumentation.uiAutomation.takeScreenshot()
                val image = Bitmap.createBitmap(screen, bounds.left, bounds.top, bounds.width(), bounds.height())
                File(context.cacheDir, "wn33-$mode-150.png").outputStream().use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
                image.recycle(); screen.recycle()
            }
            manager.updateAppWidgetOptions(id, Bundle(options).apply {
                putInt(AppWidgetManager.OPTION_APPWIDGET_MIN_WIDTH, 250)
                putInt(AppWidgetManager.OPTION_APPWIDGET_MAX_WIDTH, 250)
                putInt(AppWidgetManager.OPTION_APPWIDGET_MIN_HEIGHT, 190)
                putInt(AppWidgetManager.OPTION_APPWIDGET_MAX_HEIGHT, 190)
            })
            opened.onActivity { activity ->
                val density = activity.resources.displayMetrics.density
                view.layoutParams = FrameLayout.LayoutParams((250*density).toInt(), (190*density).toInt(), android.view.Gravity.CENTER)
            }
            waitFor("resized widget laid out") {
                var ready = false
                instrumentation.runOnMainSync { ready = !view.isLayoutRequested && texts(view).any { it.text.toString() == "Otwórz" && it.width > 0 } }
                ready
            }
            instrumentation.runOnMainSync {
                texts(view).filter { it.text.isNotEmpty() }.forEach {
                    for (line in 0 until it.layout.lineCount) assertEquals("resized text clipped: ${it.text}", 0, it.layout.getEllipsisCount(line))
                    val visible = android.graphics.Rect()
                    assertTrue(it.getGlobalVisibleRect(visible))
                    assertTrue("resized widget cuts off ${it.text}", visible.height() >= it.height)
                }
            }
            preferences.edit().putBoolean("enabled", true).commit()
            waitFor("background enabled becomes offline") { textExists(view, "Brak połączenia") }
            shell("dumpsys battery set level 78")
            waitFor("battery percentage event redraws") { textExists(view, "Telefon: 78%") }
            // CCS/1 CHECK:observe absence of RemoteViews events for125s, as required by review; not an arbitrary startup sleep.
            view.events.clear()
            val hostBefore = view.count.get()
            val publishBefore = WidgetUpdates.debugPublishes.get()
            val onUpdateBefore = WindWidgetReceiver.debugOnUpdateCalls.get()
            shell("dumpsys battery set temp 333")
            assertNull("unchanged percent/charging or WorkManager loop caused a redraw", view.events.poll(125, TimeUnit.SECONDS))
            assertEquals(hostBefore, view.count.get())
            assertEquals(publishBefore, WidgetUpdates.debugPublishes.get())
            assertNull(WidgetUpdates.debugError)
            val results = Bundle().apply {
                putInt("idleHostUpdates", view.count.get() - hostBefore)
                putInt("idlePublishes", WidgetUpdates.debugPublishes.get() - publishBefore)
                putInt("idleOnUpdateCalls", WindWidgetReceiver.debugOnUpdateCalls.get() - onUpdateBefore)
                putInt("onUpdateCallsTotal", WindWidgetReceiver.debugOnUpdateCalls.get())
                putString("idleDurationSeconds", "125")
            }
            instrumentation.sendStatus(0, results)
            preferences.edit().putBoolean("enabled", false).commit()
            waitFor("background-off after stopping") { textExists(view, "Połączenie w tle wyłączone") }
            val sender = instrumentation.addMonitor(SendClipboardActivity::class.java.name, null, false)
            try {
                instrumentation.runOnMainSync {
                    val label = texts(view).first { it.text.toString() == "Wyślij schowek" }
                    var clickable: View = label
                    while (!clickable.isClickable && clickable.parent is View) clickable = clickable.parent as View
                    assertTrue(clickable.performClick())
                }
                val activity = sender.waitForActivityWithTimeout(15_000)
                assertNotNull("widget action opens the private sender", activity)
                activity?.let { instrumentation.runOnMainSync { it.finish() } }
            } finally { instrumentation.removeMonitor(sender) }
            val main = instrumentation.addMonitor(MainActivity::class.java.name, null, false)
            try {
                opened.close()
                instrumentation.runOnMainSync {
                    var clickable: View = texts(view).first { it.text.toString() == "Otwórz" }
                    while (!clickable.isClickable && clickable.parent is View) clickable = clickable.parent as View
                    assertTrue(clickable.performClick())
                }
                val activity = main.waitForActivityWithTimeout(15_000)
                assertNotNull("Open widget action targets MainActivity", activity)
                activity?.let { instrumentation.runOnMainSync { it.finish() } }
            } finally { instrumentation.removeMonitor(main) }
        } finally {
            host.deleteAppWidgetId(id)
            host.stopListening()
            scenario?.close()
            WidgetUpdates.stop(context.applicationContext as WindApplication)
            shell("dumpsys battery reset")
            preferences.edit().putBoolean("enabled", wasEnabled).commit()
            appearance.edit().putString("theme", oldTheme).putBoolean("dynamic", oldDynamic).commit()
            shell("appwidget revokebind --package ${context.packageName} --user 0")
        }
    }
}
