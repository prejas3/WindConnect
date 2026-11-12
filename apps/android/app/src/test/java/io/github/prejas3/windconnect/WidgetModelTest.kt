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
}
