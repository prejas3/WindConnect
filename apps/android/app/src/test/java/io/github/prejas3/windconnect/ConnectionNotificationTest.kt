package io.github.prejas3.windconnect

import io.github.prejas3.windconnect.core.ConnectionState
import org.junit.Assert.*
import org.junit.Test

class ConnectionNotificationTest {
    init { StringsForTests.useEnglish() }
    @Test fun rttTicksDoNotRebuildNotificationButCapabilitiesAndRouteDo() {
        val connected = ConnectionState.Connected("PC", "trusted", 10)
        assertEquals(ConnectionService.notificationState(connected, true),
            ConnectionService.notificationState(connected.copy(roundTripMicros = 900), true))
        assertNotEquals(ConnectionService.notificationState(connected, true),
            ConnectionService.notificationState(connected, false))
        assertNotEquals(ConnectionService.notificationState(connected, true),
            ConnectionService.notificationState(connected.copy(route = "relay"), true))
        assertNotEquals(ConnectionService.notificationState(connected, true),
            ConnectionService.notificationState(ConnectionState.Disconnected("lost"), true))
    }
}
