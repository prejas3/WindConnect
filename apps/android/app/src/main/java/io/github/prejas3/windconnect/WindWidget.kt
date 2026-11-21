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
import io.github.prejas3.windconnect.ui.BrandConnectingDark
import io.github.prejas3.windconnect.ui.BrandConnectingLight
import io.github.prejas3.windconnect.ui.WindRadius
import io.github.prejas3.windconnect.ui.readable
import androidx.compose.ui.unit.DpSize
import android.text.format.DateFormat
import java.util.Date
import java.util.concurrent.atomic.AtomicInteger

class WindWidget : GlanceAppWidget() {
    // WN-160: laid out for the size it is given (small: one line of actions; large: four), no empty half.
    override val sizeMode = SizeMode.Responsive(setOf(WIDGET_SMALL, WIDGET_MEDIUM, WIDGET_LARGE))
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
            GlanceTheme(colors = widgetColors(context, snapshot.data)) { WidgetContent(context, snapshot) }
        }
    }

    /** WN-171: the picker's generated preview (Android 15+): the widget itself, connected, in the system's light/dark. */
    override suspend fun providePreview(context: Context, widgetCategory: Int) {
        val sample = WidgetSnapshot(WidgetData(name = context.getString(R.string.your_computer), status = context.getString(R.string.connected), connected = true,
            battery = 80, dynamic = false), updatedAt = System.currentTimeMillis())
        provideContent { GlanceTheme(colors = widgetColors(context, sample.data)) { WidgetContent(context, sample) } }
    }
}

/** WN-160: the app's readable scheme (secondary text and accents at a sure contrast) on the widget too. */
private fun widgetColors(context: Context, data: WidgetData): androidx.glance.color.ColorProviders {
    val dynamic = data.dynamic && Build.VERSION.SDK_INT >= 31
    val light = readable(if (dynamic) dynamicLightColorScheme(context) else BrandLight)
    val dark = readable(if (dynamic) dynamicDarkColorScheme(context) else BrandDark)
    return when (data.theme) {
        "Light" -> ColorProviders(light, light)
        "Dark" -> ColorProviders(dark, dark)
        else -> ColorProviders(light, dark)
    }
}

/** WN-171: the "connecting" dot is amber, as "searching" in the Windows tray, in the widget's light or dark. */
private fun connectingColor(theme: String): androidx.glance.unit.ColorProvider = when (theme) {
    "Light" -> androidx.glance.unit.ColorProvider(BrandConnectingLight)
    "Dark" -> androidx.glance.unit.ColorProvider(BrandConnectingDark)
    else -> androidx.glance.color.ColorProvider(day = BrandConnectingLight, night = BrandConnectingDark)
}

/** WN-171: the launcher's own widget radii from Android 12 (outer and inner), the M3 scale before. */
private fun GlanceModifier.widgetCorner(inner: Boolean): GlanceModifier =
    if (Build.VERSION.SDK_INT >= 31) cornerRadius(if (inner) android.R.dimen.system_app_widget_inner_radius else android.R.dimen.system_app_widget_background_radius)
    else cornerRadius(if (inner) WindRadius.Large else WindRadius.ExtraLarge)

// The host subtracts its own padding from the size it was given: steps by height, widths well under the 250 dp minimum.
internal val WIDGET_SMALL = DpSize(180.dp, 100.dp)
internal val WIDGET_MEDIUM = DpSize(180.dp, 150.dp)
internal val WIDGET_LARGE = DpSize(240.dp, 250.dp)
// The state dot; its corner is half its size (WindShapes.full: Glance takes a radius, not a shape).
private val WIDGET_DOT = 10.dp

/** [short]: the one-word label for the medium size at a large font (the full [label] stays the button's description). */
private class WidgetAction(val label: String, val icon: Int, val intent: Intent, val short: String = label)

@Composable
private fun WidgetContent(context: Context, snapshot: WidgetSnapshot) {
    val data = snapshot.data
    val size = LocalSize.current
    val small = size.height < WIDGET_MEDIUM.height
    val large = size.height >= WIDGET_LARGE.height
    val colors = GlanceTheme.colors
    // A large system font (the launcher uses it too) leaves no room for an icon above a two-line label in the medium size.
    val bigFont = context.resources.configuration.fontScale >= 1.3f
    val time = if (snapshot.updatedAt > 0) DateFormat.getTimeFormat(context).format(Date(snapshot.updatedAt)) else "—"
    val status = if (data.connected) context.getString(R.string.widget_connected_at, time) else data.status
    Column(GlanceModifier.fillMaxSize().appWidgetBackground().background(colors.surface)
        .widgetCorner(inner = false).padding(horizontal = 14.dp, vertical = if (large) 14.dp else 10.dp)) {
        val openIntent = Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        // Header: state dot, the computer's name, the phone's battery; tapping it opens the app (at every size).
        Column(GlanceModifier.fillMaxWidth().clickable(actionStartActivity(openIntent))) {
        // Medium (the default 4×2): the battery has its own short line, so the name keeps the full width even at a large font.
        val battery = @Composable {
            Row(GlanceModifier.semantics { contentDescription = data.batteryText }, verticalAlignment = Alignment.CenterVertically) {
                Image(ImageProvider(if (data.charging) R.drawable.ic_widget_charging else R.drawable.ic_widget_battery), null,
                    GlanceModifier.size(16.dp), colorFilter = ColorFilter.tint(colors.onSurfaceVariant))
                Spacer(GlanceModifier.width(4.dp))
                Text(data.battery?.let { "$it%" } ?: "—", style = TextStyle(color = colors.onSurface, fontSize = 12.sp, fontWeight = FontWeight.Medium), maxLines = 1)
            }
        }
        Row(GlanceModifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            val dot = when { data.connected -> colors.primary; data.connecting -> connectingColor(data.theme); else -> colors.outline }
            Box(GlanceModifier.size(WIDGET_DOT).cornerRadius(WIDGET_DOT / 2).background(dot)) {}
            Spacer(GlanceModifier.width(8.dp))
            Text(data.name, modifier = GlanceModifier.defaultWeight(), style = TextStyle(color = colors.onSurface, fontSize = 16.sp, fontWeight = FontWeight.Bold), maxLines = 1)
            if (small || large) { Spacer(GlanceModifier.width(8.dp)); battery() }
        }
        Row(GlanceModifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(status, modifier = GlanceModifier.defaultWeight().padding(start = 18.dp), style = TextStyle(color = colors.onSurfaceVariant, fontSize = 13.sp), maxLines = if (small) 1 else 2)
            if (large && !data.connected) {
                val updated = context.getString(R.string.widget_updated, time)
                Text(context.getString(R.string.widget_updated_short, time), modifier = GlanceModifier.semantics { contentDescription = updated },
                    style = TextStyle(color = colors.onSurfaceVariant, fontSize = 11.sp), maxLines = 1)
            }
        }
        if (!small && !large) Row(GlanceModifier.fillMaxWidth().padding(start = 18.dp)) { battery() }
        }
        Spacer(GlanceModifier.height(if (large) 10.dp else 6.dp))
        val clipboard = WidgetAction(context.getString(R.string.send_clipboard), R.drawable.ic_widget_clipboard, LauncherShortcuts.target(context, LauncherAction.Clipboard),
            context.getString(R.string.widget_short_clipboard))
        val open = WidgetAction(context.getString(R.string.widget_open_app), R.drawable.ic_widget_app, openIntent, context.getString(R.string.widget_short_app))
        // The actions take the rest of the height: small, the clipboard alone (the header opens the app); medium, one row;
        // large, a 2×2 grid.
        val rows = if (large) listOf(
            listOf(clipboard, WidgetAction(context.getString(R.string.shortcut_file), R.drawable.ic_widget_file, LauncherShortcuts.target(context, LauncherAction.File))),
            listOf(WidgetAction(context.getString(R.string.shortcut_touchpad), R.drawable.ic_widget_touch, LauncherShortcuts.target(context, LauncherAction.Touchpad)), open),
        ) else if (small) listOf(listOf(clipboard)) else listOf(listOf(clipboard, open))
        rows.forEachIndexed { index, row ->
            if (index > 0) Spacer(GlanceModifier.height(8.dp))
            Row(GlanceModifier.fillMaxWidth().defaultWeight()) {
                row.forEachIndexed { i, action ->
                    if (i > 0) Spacer(GlanceModifier.width(8.dp))
                    WidgetButton(action, GlanceModifier.defaultWeight().fillMaxHeight(), small, icon = large || small || !bigFont)
                }
            }
        }
    }
}

/** Small: icon beside the label on one wide button. Otherwise the icon above the label, so the label keeps the button's width. */
@Composable
private fun WidgetButton(action: WidgetAction, modifier: GlanceModifier, small: Boolean, icon: Boolean = true) {
    val colors = GlanceTheme.colors
    val base = modifier.background(colors.primaryContainer).widgetCorner(inner = true).padding(horizontal = 8.dp, vertical = 2.dp)
        .clickable(actionStartActivity(action.intent)).semantics { contentDescription = action.label }
    val label = TextStyle(color = colors.onPrimaryContainer, fontSize = 13.sp, fontWeight = FontWeight.Medium, textAlign = TextAlign.Center)
    if (small) Row(base, verticalAlignment = Alignment.CenterVertically, horizontalAlignment = Alignment.CenterHorizontally) {
        Image(ImageProvider(action.icon), null, GlanceModifier.size(18.dp), colorFilter = ColorFilter.tint(colors.onPrimaryContainer))
        Spacer(GlanceModifier.width(8.dp))
        Text(action.label, style = label, maxLines = 1)
    } else Column(base, verticalAlignment = Alignment.CenterVertically, horizontalAlignment = Alignment.CenterHorizontally) {
        if (icon) Image(ImageProvider(action.icon), null, GlanceModifier.size(18.dp), colorFilter = ColorFilter.tint(colors.onPrimaryContainer))
        Text(if (icon) action.label else action.short, style = label, maxLines = 2)
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
