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

    /**
     * WN-175: the device row's second line, "Connected · 80% charging". The battery only while the computer is connected
     * or connecting: out of reach, a percent read earlier would look current.
     */
    val line: String get() {
        val level = battery?.takeIf { connected || connecting }?.let { if (charging) str(R.string.widget_level_charging, "$it%") else "$it%" }
        return if (level == null) status else str(R.string.widget_line, status, level)
    }
}

/** WN-175: what the widget draws at a size: the device row alone with one round action, or a row of round actions. */
internal enum class WidgetLayout { Small, Medium, Large }

internal data class WidgetPlan(val layout: WidgetLayout, val actions: List<LauncherAction?>)

/**
 * WN-175 (Szymon's choice: C's compact device row with B's round icon buttons). Heights pick the layout (as the
 * responsive sizes do); a narrow widget leaves out the touchpad so every round button keeps 48 dp and its spacing.
 * `null` in [WidgetPlan.actions] is "open the app".
 */
internal fun widgetPlan(widthDp: Float, heightDp: Float): WidgetPlan {
    val layout = when { heightDp < 150f -> WidgetLayout.Small; heightDp < 250f -> WidgetLayout.Medium; else -> WidgetLayout.Large }
    val all = listOf(LauncherAction.Clipboard, LauncherAction.File, LauncherAction.Touchpad, null)
    val actions = when {
        layout == WidgetLayout.Small -> listOf(LauncherAction.Clipboard)
        widthDp < 240f -> all - LauncherAction.Touchpad
        else -> all
    }
    return WidgetPlan(layout, actions)
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
