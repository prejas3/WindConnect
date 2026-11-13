package io.github.prejas3.windconnect

import io.github.prejas3.windconnect.core.ConnectionState
import org.junit.Assert.*
import org.junit.Test

class WidgetModelTest {
    @Test fun backgroundOffNeverClaimsLiveConnected() {
        val data = widgetData(ConnectionState.Connected("PC", "secret", 1), false, 80, false)
        assertFalse(data.connected)
        assertEquals("Połączenie w tle wyłączone", data.status)
        assertEquals("Telefon: 80%", data.batteryText)
        assertEquals("PC", data.name)
        val offline = widgetData(ConnectionState.Stopped, true, 80, false, pairedName = "SZYMON")
        assertEquals("SZYMON", offline.name)
        assertFalse(offline.connected)
        assertEquals("Brak połączenia", offline.status)
    }

    @Test fun presentationIgnoresRttAndPrivateTransportDetails() {
        assertEquals(widgetData(ConnectionState.Connected("PC", "a", 1), true, 80, true),
            widgetData(ConnectionState.Connected("PC", "b", 999_999), true, 80, true))
        assertEquals(widgetData(ConnectionState.Failed("private"), true, 80, false),
            widgetData(ConnectionState.Disconnected("other private reason"), true, 80, false))
        assertEquals("PC", widgetData(ConnectionState.Connected("\u202ePC\n", "", 0), true, 1, false).name)
    }

    @Test fun percentAndChargingChangesAreDistinctButUnknownIsNotZero() {
        val data = widgetData(ConnectionState.Ready, true, null, false)
        assertEquals("Telefon: —", data.batteryText)
        assertEquals(data, widgetData(ConnectionState.Ready, true, -1, false))
        assertEquals(data, widgetData(ConnectionState.Ready, true, 101, false))
        assertNotEquals(data, widgetData(ConnectionState.Ready, true, 0, false))
        assertNotEquals(data, widgetData(ConnectionState.Ready, true, null, true))
    }

    // WN-119: the widget reads the battery as the PC's status does (BatteryReading, WN-49).
    private fun widget(level: Int, scale: Int, plugged: Int, status: Int) =
        widgetBattery(io.github.prejas3.windconnect.core.BatteryReading.from(level, scale, plugged, status))

    @Test fun batteryFollowsTheSharedModel() {
        val charging = android.os.BatteryManager.BATTERY_STATUS_CHARGING
        val full = android.os.BatteryManager.BATTERY_STATUS_FULL
        val notCharging = android.os.BatteryManager.BATTERY_STATUS_NOT_CHARGING
        val discharging = android.os.BatteryManager.BATTERY_STATUS_DISCHARGING
        val ac = android.os.BatteryManager.BATTERY_PLUGGED_AC
        assertEquals(80 to true, widget(80, 100, 0, charging))
        assertEquals(100 to true, widget(100, 100, 0, full))
        // Plugged in but paused (battery protection): charging, as the PC shows it.
        assertEquals(80 to true, widget(80, 100, ac, notCharging))
        assertEquals(41 to false, widget(41, 100, 0, discharging))
        // Unknown stays unknown: no percent and not charging, even if the status says charging.
        assertEquals(null to false, widget(-1, 100, ac, charging))
        assertEquals(null to false, widget(50, 0, 0, charging))
        assertEquals(null to false, widget(150, 100, 0, discharging))
        assertEquals(null to false, widgetBattery(null))
        assertEquals("Telefon: —", widgetData(ConnectionState.Ready, true, null, false).batteryText)
    }

    @Test fun unchangedPercentAndChargingMeanNoNewData() {
        // Temperature and voltage aren't in the reading: the same percent/charging give equal data (no widget update).
        val a = widget(1234, 2000, 0, android.os.BatteryManager.BATTERY_STATUS_DISCHARGING)
        val b = widget(1239, 2000, 0, android.os.BatteryManager.BATTERY_STATUS_DISCHARGING)
        assertEquals(61, a.first)
        assertEquals(widgetData(ConnectionState.Ready, true, a.first, a.second), widgetData(ConnectionState.Ready, true, b.first, b.second))
    }
}
