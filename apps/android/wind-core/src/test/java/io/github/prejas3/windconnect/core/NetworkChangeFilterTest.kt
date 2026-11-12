package io.github.prejas3.windconnect.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class NetworkChangeFilterTest {
    private val wifi = NetworkChangeFilter.summary("wlan0", listOf("192.168.1.20/24", "fe80::1/64"))

    @Test fun replayOfTheStartingNetworksIsNoChange() {
        val filter = NetworkChangeFilter(mapOf(100 to wifi, 101 to NetworkChangeFilter.summary("rmnet0", listOf("10.0.0.2/32"))))
        // Registration replays both networks, addresses in another order.
        assertFalse(filter.linkChanged(100, NetworkChangeFilter.summary("wlan0", listOf("fe80::1/64", "192.168.1.20/24"))))
        assertFalse(filter.linkChanged(101, NetworkChangeFilter.summary("rmnet0", listOf("10.0.0.2/32"))))
    }

    @Test fun realChangesStillCount() {
        val filter = NetworkChangeFilter(mapOf(100 to wifi))
        assertTrue("new address", filter.linkChanged(100, NetworkChangeFilter.summary("wlan0", listOf("192.168.1.21/24", "fe80::1/64"))))
        assertFalse("same again", filter.linkChanged(100, NetworkChangeFilter.summary("wlan0", listOf("192.168.1.21/24", "fe80::1/64"))))
        assertTrue("new network", filter.linkChanged(102, NetworkChangeFilter.summary("wlan0", listOf("192.168.5.7/24"))))
        assertTrue("lost", filter.lost(100))
        assertFalse("lost twice", filter.lost(100))
        assertTrue("back after loss", filter.linkChanged(100, wifi))
    }

    @Test fun changeBetweenBaselineAndReplayCounts() {
        // Wi-Fi joined after the baseline was read, before the callback was registered.
        val filter = NetworkChangeFilter(emptyMap<Int, String>())
        assertTrue(filter.linkChanged(100, wifi))
        val moved = NetworkChangeFilter(mapOf(100 to wifi))
        assertTrue(moved.linkChanged(100, NetworkChangeFilter.summary("wlan0", listOf("192.168.7.3/24"))))
    }

    @Test fun networkMovedToTheBackgroundIsNoChange() {
        // WN-111: another app drops its mobile-data request; the callback reports the still
        // connected cellular network as lost, then as available again when it asks once more.
        val cellular = NetworkChangeFilter.summary("eth0", listOf("10.0.2.15/24"))
        val filter = NetworkChangeFilter(mapOf(100 to wifi, 101 to cellular))
        assertFalse("background, same addresses", filter.lost(101, background = cellular))
        assertFalse("back in the foreground", filter.linkChanged(101, cellular))
        assertTrue("background with other addresses counts",
            filter.lost(101, background = NetworkChangeFilter.summary("eth0", listOf("10.0.2.16/24"))))
        assertTrue("really gone counts", filter.lost(100, background = null))
        assertFalse("unknown network in the background", filter.lost(102, background = cellular))
    }

    @Test fun summaryIgnoresOrderOnly() {
        assertEquals(NetworkChangeFilter.summary("wlan0", listOf("b", "a")), NetworkChangeFilter.summary("wlan0", listOf("a", "b")))
        assertTrue(NetworkChangeFilter.summary("wlan0", listOf("a")) != NetworkChangeFilter.summary("wlan1", listOf("a")))
    }
}
