package io.github.prejas3.windconnect

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withTimeoutOrNull
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * WN-149: the reconnect loop's wait (virtual time). The loop runs on the main thread; a wait that returns without
 * suspending made it spin after a successful reconnect (WN-148).
 */
class ReconnectWaitTest {
    private val old = setOf("192.168.0.10:50001")
    private val now = setOf("192.168.0.10:50077")

    /** Turns of `while (true) reconnectWait(...)` in [ms] of virtual time. */
    private suspend fun turns(active: Boolean, seen: Set<String>, tried: Set<String>, ms: Long, backoff: Long = 3_000): Int {
        var n = 0
        withTimeoutOrNull(ms) {
            // A wait that doesn't suspend never lets the timeout fire: the cap turns that spin into a failure.
            while (n < 100_000) { reconnectWait(MutableStateFlow(active), MutableStateFlow(seen), tried, backoff); n++ }
        }
        return n
    }

    @Test fun connectedWithAVisiblePcDoesNotSpin() = runTest {
        // The WN-148 case: connected after a reconnect, the PC in discovery on addresses other than `tried`.
        val a = turns(active = true, seen = now, tried = old, ms = 30_000)
        val b = turns(active = true, seen = now, tried = emptySet(), ms = 30_000)
        assertTrue("a backoff per turn, not a tight loop: $a, $b", a <= 11 && b <= 11)
    }

    @Test fun idleNeverTurnsFasterThanTheFloor() = runTest {
        // Even when every wait ends on discovery (a PC that keeps showing new addresses), at most one turn per floor.
        val n = turns(active = false, seen = now, tried = old, ms = 10_000)
        assertTrue("$n turns in 10 s", n <= 10_000 / MIN_TURN_MS.toInt() + 1)
    }

    @Test fun aSessionThatEndsEndsTheWaitAtOnce() = runTest {
        val active = MutableStateFlow(true)
        var ended = -1L
        launch { reconnectWait(active, MutableStateFlow(now), old, 60_000); ended = testScheduler.currentTime }
        advanceTimeBy(1_000)
        active.value = false
        advanceTimeBy(1)
        assertEquals(1_000L, ended)
    }

    @Test fun idleWithThePcBackOnANewAddressWakesAfterTheFloor() = runTest {
        var ended = -1L
        launch { reconnectWait(MutableStateFlow(false), MutableStateFlow(now), old, 60_000); ended = testScheduler.currentTime }
        advanceTimeBy(MIN_TURN_MS + 1)
        assertEquals(MIN_TURN_MS, ended)
    }

    @Test fun idleWithThePcUnchangedWaitsOutTheBackoff() = runTest {
        var ended = -1L
        launch { reconnectWait(MutableStateFlow(false), MutableStateFlow(old), old, 6_000); ended = testScheduler.currentTime }
        advanceTimeBy(6_001)
        assertEquals(6_000L, ended)
    }
}
