package io.github.prejas3.windconnect

import io.github.prejas3.windconnect.core.BatteryReading
import io.github.prejas3.windconnect.core.ConnectionState

internal data class WidgetData(
    val name: String = str(R.string.your_computer),
    val status: String = str(R.string.widget_background_off),
    val connected: Boolean = false,
    /** WN-160: for the status dot (connected / connecting / not). */
    val connecting: Boolean = false,
    val battery: Int? = null,
    val charging: Boolean = false,
    val theme: String = "System",
    val dynamic: Boolean = true,
) {
    val batteryText: String get() = str(if (charging) R.string.widget_battery_charging else R.string.widget_battery, battery?.let { "$it%" } ?: "—")
}

/** WN-119: the widget's percent and charging from the shared reading; none -> unknown (no percent, not charging). */
internal fun widgetBattery(reading: BatteryReading?): Pair<Int?, Boolean> = reading?.let { it.level to it.charging } ?: (null to false)

internal data class WidgetSnapshot(val data: WidgetData = WidgetData(), val updatedAt: Long = 0)

// CCS/1 RULE:RTT, raw errors, voltage and temperature do not affect the widget presentation.
// WN-160: the state in the app's words: a paired computer that isn't connected is "out of reach", none paired says so.
internal fun widgetData(state: ConnectionState, background: Boolean, percent: Int?, charging: Boolean,
    theme: String = "System", dynamic: Boolean = true, pairedName: String? = null): WidgetData {
    val connected = background && state is ConnectionState.Connected
    val status = when {
        !background -> str(R.string.widget_background_off)
        state is ConnectionState.Connected -> str(R.string.connected)
        state is ConnectionState.Connecting -> str(R.string.connecting)
        pairedName != null -> str(R.string.home_out_of_reach)
        else -> str(R.string.home_no_paired)
    }
    return WidgetData(if (state is ConnectionState.Connected) computerLabel(state.peerName)
        else pairedName?.takeIf { it.isNotBlank() }?.let(::computerLabel) ?: str(R.string.your_computer),
        status, connected, background && state is ConnectionState.Connecting, percent?.takeIf { it in 0..100 }, charging, theme, dynamic)
}
