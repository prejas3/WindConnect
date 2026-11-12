package io.github.prejas3.windconnect

import android.appwidget.AppWidgetManager
import android.content.*
import android.os.BatteryManager
import android.os.Build
import androidx.glance.appwidget.updateAll
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import java.util.concurrent.atomic.AtomicInteger

/** CCS/1 FLOW:existing connection + battery/preferences events -> deduped snapshot -> one-shot Glance update. */
internal object WidgetUpdates {
    private var scope: CoroutineScope? = null
    private var batteryReceiver: BroadcastReceiver? = null
    private var changes: SharedPreferences.OnSharedPreferenceChangeListener? = null
    private var signal: Channel<Unit>? = null
    private val mutable = MutableStateFlow(WidgetSnapshot())
    val snapshot = mutable.asStateFlow()
    val debugPublishes = AtomicInteger()
    @Volatile var debugError: String? = null
        private set

    fun installed(context: Context) = AppWidgetManager.getInstance(context)
        .getAppWidgetIds(ComponentName(context, WindWidgetReceiver::class.java)).isNotEmpty()

    @Synchronized fun start(application: WindApplication) {
        if (scope != null || !installed(application)) return
        val events = Channel<Unit>(Channel.CONFLATED)
        signal = events
        val running = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        scope = running
        val appearance = application.getSharedPreferences("appearance", Context.MODE_PRIVATE)
        val background = ConnectionService.preferences(application)
        val listener = SharedPreferences.OnSharedPreferenceChangeListener { _, key ->
            if (key in setOf("enabled", "theme", "dynamic", "peer_name")) events.trySend(Unit)
        }
        changes = listener
        appearance.registerOnSharedPreferenceChangeListener(listener)
        background.registerOnSharedPreferenceChangeListener(listener)
        application.getSharedPreferences("widget_presentation", Context.MODE_PRIVATE).registerOnSharedPreferenceChangeListener(listener)
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) { events.trySend(Unit) }
        }
        batteryReceiver = receiver
        if (Build.VERSION.SDK_INT >= 33) application.registerReceiver(receiver,
            IntentFilter(Intent.ACTION_BATTERY_CHANGED), Context.RECEIVER_NOT_EXPORTED)
        else {
            @Suppress("UnspecifiedRegisterReceiverFlag")
            application.registerReceiver(receiver, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        }
        running.launch {
            application.connections.state.map { widgetData(it, true, null, false) }
                .distinctUntilChanged().collect { events.trySend(Unit) }
        }
        running.launch {
            var published: WidgetData? = null
            for (event in events) {
                val data = read(application)
                if (data == published) continue
                if (data != mutable.value.data || mutable.value.updatedAt == 0L)
                    mutable.value = WidgetSnapshot(data, System.currentTimeMillis())
                try {
                    if (installed(application)) {
                        WindWidget().updateAll(application)
                        if (BuildConfig.DEBUG) debugPublishes.incrementAndGet()
                    }
                    published = data
                } catch (error: Exception) {
                    if (error is CancellationException) throw error
                    if (BuildConfig.DEBUG) debugError = error.javaClass.simpleName
                }
            }
        }
        events.trySend(Unit)
    }

    private fun read(application: WindApplication): WidgetData {
        val battery = application.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        val level = battery?.getIntExtra(BatteryManager.EXTRA_LEVEL, -1) ?: -1
        val scale = battery?.getIntExtra(BatteryManager.EXTRA_SCALE, -1) ?: -1
        val percent = if (level >= 0 && scale > 0) (level * 100 / scale) else null
        val status = battery?.getIntExtra(BatteryManager.EXTRA_STATUS, -1)
        val appearance = application.getSharedPreferences("appearance", Context.MODE_PRIVATE)
        return widgetData(application.connections.state.value, ConnectionService.enabled(application), percent,
            status == BatteryManager.BATTERY_STATUS_CHARGING || status == BatteryManager.BATTERY_STATUS_FULL,
            appearance.getString("theme", "System") ?: "System", appearance.getBoolean("dynamic", true),
            application.getSharedPreferences("widget_presentation", Context.MODE_PRIVATE).getString("peer_name", null))
    }

    fun current(application: WindApplication) = WidgetSnapshot(read(application), System.currentTimeMillis())

    @Synchronized fun stop(application: WindApplication) {
        scope?.cancel(); scope = null
        signal?.close(); signal = null
        batteryReceiver?.let { application.unregisterReceiver(it) }; batteryReceiver = null
        changes?.let {
            application.getSharedPreferences("appearance", Context.MODE_PRIVATE).unregisterOnSharedPreferenceChangeListener(it)
            ConnectionService.preferences(application).unregisterOnSharedPreferenceChangeListener(it)
            application.getSharedPreferences("widget_presentation", Context.MODE_PRIVATE).unregisterOnSharedPreferenceChangeListener(it)
        }
        changes = null
    }
}
