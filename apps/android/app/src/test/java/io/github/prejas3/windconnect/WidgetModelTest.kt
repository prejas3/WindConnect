package io.github.prejas3.windconnect

import io.github.prejas3.windconnect.core.ConnectionState
import org.junit.Assert.*
import org.junit.Test

class WidgetModelTest {
    init { StringsForTests.useEnglish() }
    @Test fun backgroundOffNeverClaimsLiveConnected() {
        val data = widgetData(ConnectionState.Connected("PC", "secret", 1), false, 80, false)
        assertFalse(data.connected)
        assertEquals("Background connection off", data.status)
        assertEquals("Phone: 80%", data.batteryText)
        assertEquals("PC", data.name)
        val offline = widgetData(ConnectionState.Stopped, true, 80, false, pairedName = "SZYMON")
        assertEquals("SZYMON", offline.name)
        assertFalse(offline.connected)
        // WN-160: the app's words: a paired computer that isn't connected is out of reach; none paired says so.
        assertEquals("Computer out of reach", offline.status)
        assertEquals("No paired computer", widgetData(ConnectionState.Stopped, true, 80, false).status)
        assertTrue(widgetData(ConnectionState.Connecting, true, 80, false, pairedName = "SZYMON").connecting)
        assertFalse(widgetData(ConnectionState.Connecting, false, 80, false, pairedName = "SZYMON").connecting)
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
        assertEquals("Phone: —", data.batteryText)
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
        assertEquals("Phone: —", widgetData(ConnectionState.Ready, true, null, false).batteryText)
    }

    @Test fun unchangedPercentAndChargingMeanNoNewData() {
        // Temperature and voltage aren't in the reading: the same percent/charging give equal data (no widget update).
        val a = widget(1234, 2000, 0, android.os.BatteryManager.BATTERY_STATUS_DISCHARGING)
        val b = widget(1239, 2000, 0, android.os.BatteryManager.BATTERY_STATUS_DISCHARGING)
        assertEquals(61, a.first)
        assertEquals(widgetData(ConnectionState.Ready, true, a.first, a.second), widgetData(ConnectionState.Ready, true, b.first, b.second))
    }

    /** WN-175: the device row's second line; the battery only while the computer is there (a stale percent would mislead). */
    @Test fun deviceLineShowsStateAndBatteryOnlyWhenLive() {
        assertEquals("Connected · 80% charging", widgetData(ConnectionState.Connected("PC", "k", 1), true, 80, true).line)
        assertEquals("Connected · 64%", widgetData(ConnectionState.Connected("PC", "k", 1), true, 64, false).line)
        assertEquals("Connecting… · 64%", widgetData(ConnectionState.Connecting, true, 64, false, pairedName = "PC").line)
        assertEquals("Computer out of reach", widgetData(ConnectionState.Stopped, true, 64, false, pairedName = "PC").line)
        assertEquals("Connected", widgetData(ConnectionState.Connected("PC", "k", 1), true, null, false).line)
        StringsForTests.usePolish()
        try { assertTrue(widgetData(ConnectionState.Connected("PC", "k", 1), true, 80, true).line.endsWith("80%, ładuje się")) } // i18n: pl
        finally { StringsForTests.useEnglish() }
    }

    /** WN-175: C's row with B's round actions at every size; narrow widths drop the touchpad to keep 48 dp buttons. */
    @Test fun sizesPickTheLayoutAndTheRoundActions() {
        assertEquals(WidgetPlan(WidgetLayout.Small, listOf(LauncherAction.Clipboard)), widgetPlan(320f, 110f))
        assertEquals(WidgetLayout.Medium, widgetPlan(320f, 180f).layout)
        assertEquals(listOf(LauncherAction.Clipboard, LauncherAction.File, LauncherAction.Touchpad, null), widgetPlan(320f, 180f).actions)
        assertEquals(listOf(LauncherAction.Clipboard, LauncherAction.File, null), widgetPlan(200f, 180f).actions)
        assertEquals(WidgetLayout.Large, widgetPlan(320f, 330f).layout)
        // Every round button (48 dp) and its gaps fit the narrowest allowed width (180 dp minus 28 dp padding).
        for (width in listOf(180f, 239f, 240f, 360f)) {
            val n = widgetPlan(width, 180f).actions.size
            assertTrue("$n buttons at $width dp", n * 48 + (n - 1) * 4 <= width - 28)
        }
    }
}
