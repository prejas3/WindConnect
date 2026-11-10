package io.github.prejas3.windconnect.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** WN-83: the multicast lock follows discovery need, not the app's life. */
class MulticastPolicyTest {
    private class FakeLock : MulticastSwitch {
        override var isHeld = false
        var acquired = 0
        override fun acquire() { isHeld = true; acquired++ }
        override fun release() { isHeld = false }
    }

    private val lock = FakeLock()
    private val policy = MulticastPolicy(lock, window = 1_000)

    @Test fun startLooksForAWindowThenLetsGo() {
        assertFalse("nothing is held before discovery starts", lock.isHeld)
        policy.wake(0)
        assertTrue(lock.isHeld)
        assertEquals(1_000L, policy.nextCheck(0))
        policy.tick(999)
        assertTrue(lock.isHeld)
        policy.tick(1_000)
        assertFalse("released after the window, even with no session", lock.isHeld)
        assertNull(policy.nextCheck(1_000))
    }

    @Test fun anActiveSessionHoldsNoLockEvenAfterANetworkChange() {
        policy.wake(0)
        policy.connected(100, true)
        assertFalse("released as soon as the session is up", lock.isHeld)
        policy.wake(200)
        assertFalse("QUIC migration, not multicast, brings the session back", lock.isHeld)
        assertNull(policy.nextCheck(200))
    }

    @Test fun aNetworkChangeWithoutASessionLooksAgain() {
        policy.wake(0)
        policy.tick(1_000)
        assertFalse(lock.isHeld)
        policy.wake(5_000)
        assertTrue("discovery works again after the change, as in WN-16", lock.isHeld)
        assertEquals(1_000L, policy.nextCheck(5_000))
        policy.wake(5_500)
        assertEquals("a second change extends the window", 1_000L, policy.nextCheck(5_500))
        policy.tick(6_500)
        assertFalse(lock.isHeld)
    }

    @Test fun aLostSessionLooksForTheWindow() {
        policy.connected(0, true)
        policy.connected(10_000, false)
        assertTrue(lock.isHeld)
        policy.tick(11_000)
        assertFalse(lock.isHeld)
    }

    @Test fun anOpenDiscoveryScreenHoldsUntilEveryOneCloses() {
        policy.connected(0, true)
        policy.screen(0, true)
        policy.screen(0, true)
        assertTrue("a SAS window or device list holds even with a session", lock.isHeld)
        assertNull("no timer while a screen is open", policy.nextCheck(0))
        policy.tick(60_000)
        policy.screen(60_000, false)
        assertTrue(lock.isHeld)
        policy.screen(60_000, false)
        assertFalse(lock.isHeld)
        policy.screen(60_000, false)
        assertFalse("an extra close does not go negative", lock.isHeld)
        policy.screen(60_000, true)
        assertTrue(lock.isHeld)
    }

    @Test fun repeatedEventsDoNotReacquire() {
        policy.wake(0)
        policy.wake(10)
        policy.connected(20, false)
        assertEquals(1, lock.acquired)
    }

    @Test fun closeReleasesAndStaysReleased() {
        policy.wake(0)
        policy.close()
        assertFalse(lock.isHeld)
        policy.wake(10)
        policy.screen(10, true)
        assertFalse(lock.isHeld)
    }
}
