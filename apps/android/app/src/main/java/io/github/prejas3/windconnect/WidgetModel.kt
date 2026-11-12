package io.github.prejas3.windconnect

import io.github.prejas3.windconnect.core.ConnectionState

internal data class WidgetData(
    val name: String = "Twój komputer",
    val status: String = "Połączenie w tle wyłączone",
    val connected: Boolean = false,
    val battery: Int? = null,
    val charging: Boolean = false,
    val theme: String = "System",
    val dynamic: Boolean = true,
) {
    val batteryText: String get() = "Telefon: ${battery?.let { "$it%" } ?: "—"}${if (charging) " · ładowanie" else ""}"
}

internal data class WidgetSnapshot(val data: WidgetData = WidgetData(), val updatedAt: Long = 0)

// CCS/1 RULE:RTT, raw errors, voltage and temperature do not affect the widget presentation.
internal fun widgetData(state: ConnectionState, background: Boolean, percent: Int?, charging: Boolean,
    theme: String = "System", dynamic: Boolean = true, pairedName: String? = null): WidgetData {
    val connected = background && state is ConnectionState.Connected
    val status = when {
        !background -> "Połączenie w tle wyłączone"
        state is ConnectionState.Connected -> "Połączono"
        state is ConnectionState.Connecting -> "Łączenie…"
        else -> "Brak połączenia"
    }
    return WidgetData(if (state is ConnectionState.Connected) computerLabel(state.peerName)
        else pairedName?.takeIf { it.isNotBlank() }?.let(::computerLabel) ?: "Twój komputer",
        status, connected, percent?.takeIf { it in 0..100 }, charging, theme, dynamic)
}
