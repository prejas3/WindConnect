package io.github.prejas3.windconnect

import android.appwidget.AppWidgetHost
import android.appwidget.AppWidgetHostView
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProviderInfo
import android.content.ComponentName
import android.content.Context
import android.graphics.Bitmap
import android.os.Bundle
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import android.widget.TextView
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assume.assumeTrue
import org.junit.Test
import java.io.ByteArrayOutputStream

/**
 * WN-160: the widget in a real host (WidgetHostTest's way) at a small, medium and large size, in the dark Material You
 * scheme. Not a check: on demand with -e uiShots true; PNGs to /data/local/tmp/wn160 (the app is uninstalled after the run).
 */
class UiWidgetShotsTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val tag = InstrumentationRegistry.getArguments().getString("shotTag") ?: "shot"

    private fun shell(command: String): String = ParcelFileDescriptor.AutoCloseInputStream(
        instrumentation.uiAutomation.executeShellCommand(command)).use { String(it.readBytes()) }

    private fun save(name: String, bitmap: Bitmap) {
        shell("mkdir -p /data/local/tmp/wn160")
        val bytes = ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
        val pipes = instrumentation.uiAutomation.executeShellCommandRw("dd of=/data/local/tmp/wn160/$tag-$name.png")
        ParcelFileDescriptor.AutoCloseOutputStream(pipes[1]).use { it.write(bytes) }
        ParcelFileDescriptor.AutoCloseInputStream(pipes[0]).use { it.readBytes() }
    }

    private fun texts(view: View): List<TextView> = when (view) {
        is TextView -> listOf(view)
        is ViewGroup -> (0 until view.childCount).flatMap { texts(view.getChildAt(it)) }
        else -> emptyList()
    }

    @Test fun widgetSizes() {
        assumeTrue("screenshots only on demand", InstrumentationRegistry.getArguments().getString("uiShots") == "true")
        val preferences = ConnectionService.preferences(context)
        val appearance = context.getSharedPreferences("appearance", Context.MODE_PRIVATE)
        val wasEnabled = preferences.getBoolean("enabled", false)
        preferences.edit().putBoolean("enabled", true).commit()
        appearance.edit().putString("theme", "Dark").putBoolean("dynamic", true).commit()
        shell("appwidget grantbind --package ${context.packageName} --user 0")
        shell("dumpsys battery set level 68")
        var fontScale = 1f
        val host = object : AppWidgetHost(context, 330160) {
            override fun onCreateView(context: Context, appWidgetId: Int, appWidget: AppWidgetProviderInfo) = AppWidgetHostView(
                context.createConfigurationContext(android.content.res.Configuration(context.resources.configuration).apply { this.fontScale = fontScale }))
        }
        val manager = AppWidgetManager.getInstance(context)
        val scenario = ActivityScenario.launch(MainActivity::class.java)
        val ids = mutableListOf<Int>()
        try {
            host.startListening()
            // The first widget of a fresh process stays on its loading layout in this host: a warm-up one first, not saved.
            for ((label, size) in listOf("warmup" to (320 to 190), "small" to (250 to 110), "medium" to (320 to 190), "large" to (360 to 330), "medium-font150" to (250 to 190))) {
                val (w, h) = size
                fontScale = if (label.endsWith("font150")) 1.5f else 1f
                // As on a phone: the system font scale reaches the launcher and the app (the widget reads it).
                shell("settings put system font_scale $fontScale")
                SystemClock.sleep(1000)
                val id = host.allocateAppWidgetId().also { ids += it }
                check(manager.bindAppWidgetIdIfAllowed(id, ComponentName(context, WindWidgetReceiver::class.java)))
                val info = manager.getAppWidgetInfo(id)
                manager.updateAppWidgetOptions(id, Bundle().apply {
                    putInt(AppWidgetManager.OPTION_APPWIDGET_MIN_WIDTH, w); putInt(AppWidgetManager.OPTION_APPWIDGET_MAX_WIDTH, w)
                    putInt(AppWidgetManager.OPTION_APPWIDGET_MIN_HEIGHT, h); putInt(AppWidgetManager.OPTION_APPWIDGET_MAX_HEIGHT, h)
                })
                lateinit var view: AppWidgetHostView
                scenario.onActivity { activity ->
                    view = host.createView(activity, id, info)
                    val density = activity.resources.displayMetrics.density
                    activity.setContentView(FrameLayout(activity).apply {
                        setBackgroundColor(0xFF3B4A55.toInt())
                        addView(view, FrameLayout.LayoutParams((w * density).toInt(), (h * density).toInt(), android.view.Gravity.CENTER))
                    })
                }
                val loading = context.getString(R.string.widget_name)
                val deadline = SystemClock.elapsedRealtime() + 90_000
                while (SystemClock.elapsedRealtime() < deadline) {
                    var drawn = false
                    instrumentation.runOnMainSync { drawn = texts(view).let { t -> t.isNotEmpty() && t.none { it.text.toString() == loading } } }
                    if (drawn) break
                    SystemClock.sleep(200)
                }
                SystemClock.sleep(1500) // the host cross-fades from the loading layout
                instrumentation.waitForIdleSync()
                val location = IntArray(2)
                instrumentation.runOnMainSync { view.getLocationOnScreen(location) }
                val screen = instrumentation.uiAutomation.takeScreenshot()
                if (label != "warmup") save("widget-$label", Bitmap.createBitmap(screen, location[0], location[1], view.width, view.height))
            }
        } finally {
            shell("settings put system font_scale 1.0")
            ids.forEach { host.deleteAppWidgetId(it) }
            host.stopListening()
            scenario.close()
            WidgetUpdates.stop(context.applicationContext as WindApplication)
            shell("dumpsys battery reset")
            preferences.edit().putBoolean("enabled", wasEnabled).commit()
            shell("appwidget revokebind --package ${context.packageName} --user 0")
        }
    }
}
