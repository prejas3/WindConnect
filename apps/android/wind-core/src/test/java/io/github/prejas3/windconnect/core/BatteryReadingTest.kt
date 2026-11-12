package io.github.prejas3.windconnect.core

import android.os.BatteryManager
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class BatteryReadingTest {
    private val discharging = BatteryManager.BATTERY_STATUS_DISCHARGING

    @Test fun percentAsTheStatusBarShowsIt() {
        assertEquals(BatteryReading(82, false), BatteryReading.from(82, 100, 0, discharging))
        assertEquals("rounded down, as SettingsLib does", BatteryReading(49, false), BatteryReading.from(127, 255, 0, discharging))
        assertEquals(BatteryReading(100, false), BatteryReading.from(100, 100, 0, discharging))
    }

    @Test fun pluggedChargingOrFullIsCharging() {
        assertEquals(true, BatteryReading.from(40, 100, BatteryManager.BATTERY_PLUGGED_USB, discharging)!!.charging)
        assertEquals(true, BatteryReading.from(40, 100, 0, BatteryManager.BATTERY_STATUS_CHARGING)!!.charging)
        assertEquals(true, BatteryReading.from(100, 100, 0, BatteryManager.BATTERY_STATUS_FULL)!!.charging)
        assertEquals(false, BatteryReading.from(40, 100, 0, BatteryManager.BATTERY_STATUS_NOT_CHARGING)!!.charging)
    }

    @Test fun unknownIsNoReading() {
        assertNull(BatteryReading.from(-1, 100, 0, discharging))
        assertNull(BatteryReading.from(50, -1, 0, discharging))
        assertNull(BatteryReading.from(50, 0, 0, discharging))
        assertNull(BatteryReading.from(101, 100, 0, discharging))
        assertNull(BatteryReading.from(null))
    }
}
