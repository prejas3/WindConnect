package io.github.prejas3.windconnect

import android.appwidget.AppWidgetManager
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.datastore.preferences.core.Preferences
import androidx.glance.*
import androidx.glance.action.clickable
import androidx.glance.appwidget.*
import androidx.glance.appwidget.action.actionStartActivity
import androidx.glance.layout.*
import androidx.glance.material3.ColorProviders
import androidx.glance.semantics.contentDescription
import androidx.glance.semantics.semantics
import androidx.glance.text.*
import io.github.prejas3.windconnect.ui.BrandDark
import io.github.prejas3.windconnect.ui.BrandLight
import android.text.format.DateFormat
import java.util.Date
import java.util.concurrent.atomic.AtomicInteger

class WindWidget : GlanceAppWidget() {
    override val sizeMode = SizeMode.Single
    override suspend fun provideGlance(context: Context, id: GlanceId) {
        WidgetUpdates.start(context.applicationContext as WindApplication)
        val initial = WidgetUpdates.current(context.applicationContext as WindApplication)
        provideContent {
            // CCS/1 FLOW:one Glance state revision per publication; do not also collect the model flow and redraw twice.
            val revision = currentState<Preferences>()[WIDGET_RENDER_REVISION] ?: 0L
            val snapshot = remember(revision) {
                val latest = WidgetUpdates.snapshot.value
                if (latest.updatedAt == 0L || !WidgetUpdates.installed(context)) initial else latest
            }
            val data = snapshot.data
            val light = if (data.dynamic && Build.VERSION.SDK_INT >= 31) dynamicLightColorScheme(context) else BrandLight
            val dark = if (data.dynamic && Build.VERSION.SDK_INT >= 31) dynamicDarkColorScheme(context) else BrandDark
            val colors = when (data.theme) {
                "Light" -> ColorProviders(light, light)
                "Dark" -> ColorProviders(dark, dark)
                else -> ColorProviders(light, dark)
            }
            GlanceTheme(colors = colors) { WidgetContent(context, snapshot) }
        }
    }
}

@Composable
private fun WidgetContent(context: Context, snapshot: WidgetSnapshot) {
    val data = snapshot.data
    val time = if (snapshot.updatedAt > 0) DateFormat.getTimeFormat(context).format(Date(snapshot.updatedAt)) else "—"
    val status = if (data.connected) context.getString(R.string.widget_connected_at, time) else data.status
    Column(GlanceModifier.fillMaxSize().appWidgetBackground().background(GlanceTheme.colors.surface)
        .cornerRadius(24.dp).padding(8.dp)) {
        Text(data.name, style = TextStyle(color = GlanceTheme.colors.onSurface, fontSize = 16.sp, fontWeight = FontWeight.Bold), maxLines = 1)
        Text(status, style = TextStyle(color = GlanceTheme.colors.onSurfaceVariant, fontSize = 12.sp), maxLines = 2)
        Row(GlanceModifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            val battery = context.getString(R.string.widget_battery, data.battery?.let { "$it%" } ?: "—") + if (data.charging) " ⚡" else ""
            Text(battery, modifier = GlanceModifier.defaultWeight().semantics { contentDescription = data.batteryText },
                style = TextStyle(color = GlanceTheme.colors.onSurfaceVariant, fontSize = 11.sp), maxLines = 1)
            val updated = context.getString(R.string.widget_updated, time)
            Text(context.getString(R.string.widget_updated_short, time), modifier = GlanceModifier.semantics { contentDescription = updated },
                style = TextStyle(color = GlanceTheme.colors.onSurfaceVariant, fontSize = 10.sp), maxLines = 1)
        }
        Spacer(GlanceModifier.height(4.dp))
        Row(GlanceModifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            WidgetAction(context.getString(R.string.send_clipboard), GlanceModifier.defaultWeight(), Intent(context, SendClipboardActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK))
            Spacer(GlanceModifier.width(8.dp))
            WidgetAction(context.getString(R.string.open), GlanceModifier.defaultWeight(), Intent(context, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP))
        }
    }
}

@Composable
private fun WidgetAction(label: String, modifier: GlanceModifier, intent: Intent) {
    Box(modifier.height(48.dp).background(GlanceTheme.colors.primaryContainer).cornerRadius(12.dp)
        .clickable(actionStartActivity(intent)).semantics { contentDescription = label }, contentAlignment = Alignment.Center) {
        Text(label, style = TextStyle(color = GlanceTheme.colors.onPrimaryContainer, fontSize = 12.sp, fontWeight = FontWeight.Medium), maxLines = 2)
    }
}

class WindWidgetReceiver : GlanceAppWidgetReceiver() {
    override val glanceAppWidget = WindWidget()
    override fun onEnabled(context: Context) {
        super.onEnabled(context)
        WidgetUpdates.start(context.applicationContext as WindApplication)
    }
    override fun onUpdate(context: Context, appWidgetManager: AppWidgetManager, appWidgetIds: IntArray) {
        if (BuildConfig.DEBUG) debugOnUpdateCalls.incrementAndGet()
        WidgetUpdates.start(context.applicationContext as WindApplication)
        // CCS/1 WHY:WorkManager component PACKAGE_CHANGED can cause repeated APPWIDGET_UPDATE. Only initial IDs need a session here; data changes use WidgetUpdates.
        val initial = synchronized(requested) { appWidgetIds.filter { requested.add(it) }.toIntArray() }
        if (initial.isNotEmpty()) super.onUpdate(context, appWidgetManager, initial)
    }
    override fun onAppWidgetOptionsChanged(context: Context, appWidgetManager: AppWidgetManager, appWidgetId: Int, newOptions: Bundle) {
        WidgetUpdates.start(context.applicationContext as WindApplication)
        super.onAppWidgetOptionsChanged(context, appWidgetManager, appWidgetId, newOptions)
    }
    override fun onDeleted(context: Context, appWidgetIds: IntArray) {
        synchronized(requested) { appWidgetIds.forEach { requested.remove(it) } }
        super.onDeleted(context, appWidgetIds)
    }
    override fun onDisabled(context: Context) {
        super.onDisabled(context)
        WidgetUpdates.stop(context.applicationContext as WindApplication)
        synchronized(requested) { requested.clear() }
    }
    companion object {
        private val requested = mutableSetOf<Int>()
        val debugOnUpdateCalls = AtomicInteger()
    }
}
