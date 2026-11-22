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
    // WN-160/WN-175: laid out for the size it is given (small: the device row and one round action; medium: a row of
    // round actions; large: labelled round actions and the last update), no empty half.
    override val sizeMode = SizeMode.Responsive(WIDGET_SIZES)
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

// The host subtracts its own padding from the size it was given: steps by height (small, medium, large) and by width
// (narrow: three round actions, wide: four), widths well under the 250 dp minimum.
internal val WIDGET_SMALL = DpSize(180.dp, 100.dp)
internal val WIDGET_MEDIUM = DpSize(180.dp, 150.dp)
internal val WIDGET_LARGE = DpSize(180.dp, 250.dp)
private val WIDGET_SIZES = setOf(WIDGET_SMALL, WIDGET_MEDIUM, WIDGET_LARGE) +
    setOf(WIDGET_SMALL, WIDGET_MEDIUM, WIDGET_LARGE).map { DpSize(240.dp, it.height) }
// The device tile and its state badge (WN-175: C's row); the badge sits in a ring of the widget's surface.
private val WIDGET_DEVICE = 44.dp
private val WIDGET_BADGE = 14.dp
private val WIDGET_DOT = 10.dp
// B's round actions: 48 dp touch targets at least; larger where the widget has the room.
private val WIDGET_ROUND = 48.dp
private val WIDGET_ROUND_LARGE = 56.dp

private class WidgetAction(val label: String, val short: String, val icon: Int, val intent: Intent)

@Composable
private fun WidgetContent(context: Context, snapshot: WidgetSnapshot) {
    val data = snapshot.data
    val size = LocalSize.current
    val plan = widgetPlan(size.width.value, size.height.value)
    val colors = GlanceTheme.colors
    val time = if (snapshot.updatedAt > 0) DateFormat.getTimeFormat(context).format(Date(snapshot.updatedAt)) else "—"
    val openIntent = Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
    val actions = plan.actions.map { action ->
        when (action) {
            LauncherAction.Clipboard -> WidgetAction(context.getString(R.string.send_clipboard), context.getString(R.string.widget_short_clipboard),
                R.drawable.ic_widget_clipboard, LauncherShortcuts.target(context, action))
            LauncherAction.File -> WidgetAction(context.getString(R.string.shortcut_file), context.getString(R.string.widget_short_file),
                R.drawable.ic_widget_file, LauncherShortcuts.target(context, action))
            LauncherAction.Touchpad -> WidgetAction(context.getString(R.string.shortcut_touchpad), context.getString(R.string.widget_short_touchpad),
                R.drawable.ic_widget_touch, LauncherShortcuts.target(context, action))
            null -> WidgetAction(context.getString(R.string.widget_open_app), context.getString(R.string.widget_short_app), R.drawable.ic_widget_app, openIntent)
        }
    }
    // Out of reach (or background off) the actions go neutral; they still open what they open (the app then says why).
    val live = data.connected || data.connecting
    Column(GlanceModifier.fillMaxSize().appWidgetBackground().background(colors.surface).widgetCorner(inner = false).padding(14.dp)) {
        // The device row: computer tile with its state badge, name, "state · battery"; tapping it opens the app.
        Row(GlanceModifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Row(GlanceModifier.defaultWeight().clickable(actionStartActivity(openIntent))
                .semantics { contentDescription = "${data.name}, ${data.line}" }, verticalAlignment = Alignment.CenterVertically) {
                Box(GlanceModifier.size(WIDGET_DEVICE + 4.dp), contentAlignment = Alignment.BottomEnd) {
                    Box(GlanceModifier.size(WIDGET_DEVICE).cornerRadius(WindRadius.Medium).background(colors.surfaceVariant), contentAlignment = Alignment.Center) {
                        Image(ImageProvider(R.drawable.ic_widget_pc), null, GlanceModifier.size(22.dp), colorFilter = ColorFilter.tint(colors.onSurface))
                    }
                    val dot = when { data.connected -> colors.primary; data.connecting -> connectingColor(data.theme); else -> colors.outline }
                    Box(GlanceModifier.size(WIDGET_BADGE).cornerRadius(WIDGET_BADGE / 2).background(colors.surface), contentAlignment = Alignment.Center) {
                        Box(GlanceModifier.size(WIDGET_DOT).cornerRadius(WIDGET_DOT / 2).background(dot)) {}
                    }
                }
                Spacer(GlanceModifier.width(10.dp))
                Column(GlanceModifier.defaultWeight()) {
                    Text(data.name, style = TextStyle(color = colors.onSurface, fontSize = 16.sp, fontWeight = FontWeight.Bold), maxLines = if (plan.layout == WidgetLayout.Small) 1 else 2)
                    Text(data.line, style = TextStyle(color = colors.onSurfaceVariant, fontSize = 13.sp), maxLines = if (plan.layout == WidgetLayout.Small) 1 else 2)
                }
            }
            // Small (4×1): the clipboard as the one round action at the row's end.
            if (plan.layout == WidgetLayout.Small) {
                Spacer(GlanceModifier.width(8.dp))
                RoundAction(actions.first(), WIDGET_ROUND, live, label = false)
            }
        }
        if (plan.layout != WidgetLayout.Small) {
            Spacer(GlanceModifier.defaultWeight())
            val large = plan.layout == WidgetLayout.Large
            Row(GlanceModifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                actions.forEachIndexed { i, action ->
                    if (i > 0) Spacer(GlanceModifier.defaultWeight())
                    RoundAction(action, if (large) WIDGET_ROUND_LARGE else WIDGET_ROUND, live, label = large)
                }
            }
            if (large) {
                Spacer(GlanceModifier.defaultWeight())
                val updated = context.getString(R.string.widget_updated, time)
                Text(updated, style = TextStyle(color = colors.onSurfaceVariant, fontSize = 12.sp), maxLines = 1)
            }
        }
    }
}

/** B's round icon button; its name is the description (TalkBack), and the short label under it in the large size. */
@Composable
private fun RoundAction(action: WidgetAction, size: androidx.compose.ui.unit.Dp, live: Boolean, label: Boolean) {
    val colors = GlanceTheme.colors
    val fill = if (live) colors.primaryContainer else colors.surfaceVariant
    val ink = if (live) colors.onPrimaryContainer else colors.onSurfaceVariant
    Column(GlanceModifier.clickable(actionStartActivity(action.intent)).semantics { contentDescription = action.label },
        horizontalAlignment = Alignment.CenterHorizontally) {
        Box(GlanceModifier.size(size).cornerRadius(size / 2).background(fill), contentAlignment = Alignment.Center) {
            Image(ImageProvider(action.icon), null, GlanceModifier.size(22.dp), colorFilter = ColorFilter.tint(ink))
        }
        if (label) {
            Spacer(GlanceModifier.height(4.dp))
            Text(action.short, style = TextStyle(color = colors.onSurface, fontSize = 12.sp, textAlign = TextAlign.Center), maxLines = 1)
        }
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
