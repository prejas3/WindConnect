package io.github.prejas3.windconnect.core

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.BatteryManager
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * The phone's battery as WindConnect reports it (WN-49, PROTOCOL.md §9.4): percent and whether it charges.
 * CCS/1 RULE:one normalisation for every reader (the PC status, the widget later): FULL counts as charging;
 * an unknown level is no reading (nothing is sent).
 */
data class BatteryReading(val level: Int, val charging: Boolean) {
    companion object {
        /** From an `ACTION_BATTERY_CHANGED` intent; null when the level or scale is missing. */
        fun from(intent: Intent?): BatteryReading? = intent?.let {
            from(
                it.getIntExtra(BatteryManager.EXTRA_LEVEL, -1),
                it.getIntExtra(BatteryManager.EXTRA_SCALE, -1),
                it.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0),
                it.getIntExtra(BatteryManager.EXTRA_STATUS, BatteryManager.BATTERY_STATUS_UNKNOWN),
            )
        }

        /** The same from raw extras (testable without Android): percent as the status bar shows it. */
        fun from(level: Int, scale: Int, plugged: Int, status: Int): BatteryReading? {
            if (level < 0 || scale <= 0 || level > scale) return null
            val charging = plugged != 0 || status == BatteryManager.BATTERY_STATUS_CHARGING || status == BatteryManager.BATTERY_STATUS_FULL
            return BatteryReading(level * 100 / scale, charging)
        }
    }
}

/**
 * CCS/1 RULE:event-driven, no polling: the sticky `ACTION_BATTERY_CHANGED` receiver is registered only while a
 * session is live (WN-49); it never wakes the phone itself. Equal readings are dropped (the intent also fires
 * for voltage and temperature).
 */
internal class BatteryReporter(context: Context) {
    private val context = context.applicationContext
    private val mutable = MutableStateFlow<BatteryReading?>(null)
    val readings: StateFlow<BatteryReading?> = mutable.asStateFlow()
    private var registered = false
    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            BatteryReading.from(intent)?.let { mutable.value = it }
        }
    }

    @Synchronized fun start() {
        if (registered) return
        // A protected system broadcast: no export flag needed. The sticky intent is the current state.
        BatteryReading.from(context.registerReceiver(receiver, IntentFilter(Intent.ACTION_BATTERY_CHANGED)))?.let { mutable.value = it }
        registered = true
    }

    @Synchronized fun stop() {
        if (!registered) return
        registered = false
        try { context.unregisterReceiver(receiver) } catch (_: IllegalArgumentException) {}
    }
}
