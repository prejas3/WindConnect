package io.github.prejas3.windconnect

import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.delay
import kotlinx.coroutines.withTimeoutOrNull

/**
 * WN-142: the background reconnect's wait between attempts. Waits up to [ms], but ends at once when
 * the paired PC comes back in discovery: it reappears after being gone, or shows up on addresses
 * other than [tried] (the ones the last attempt used, e.g. a PC app that restarted on a new port).
 * The same addresses staying put don't end the wait, so a PC that answers BUSY or refuses us isn't
 * retried in a loop (WC-28). Discovery only wakes the loop; the connect still needs the pinned key.
 *
 * [addresses] is the PC's current address set as discovery sees it (empty when not seen).
 * True when the PC came back, false when the time ran out.
 */
/** WN-149: the shortest wait between two attempts, whatever ends it: the loop can never spin. */
internal const val MIN_TURN_MS = 500L

/**
 * WN-149: one wait of the background reconnect loop (on the main thread). While a session is up or being set up
 * ([active] true), discovery can't end the wait: it lasts until the session ends or [ms] passes. Otherwise a visible PC
 * whose addresses differ from [tried] would end every wait at once and the loop would spin, freezing the UI. When idle,
 * at least [MIN_TURN_MS], then [awaitPcReturn] for the rest of [ms] ([addresses] null: a plain wait).
 */
internal suspend fun reconnectWait(active: Flow<Boolean>, addresses: Flow<Set<String>>?, tried: Set<String>, ms: Long) {
    if (active.first()) {
        withTimeoutOrNull(ms) { active.first { !it } }
        return
    }
    val floor = MIN_TURN_MS.coerceAtMost(ms)
    delay(floor)
    if (addresses == null) delay(ms - floor) else awaitPcReturn(addresses, tried, ms - floor)
}

internal suspend fun awaitPcReturn(addresses: Flow<Set<String>>, tried: Set<String>, ms: Long): Boolean {
    var gone = tried.isEmpty()
    return withTimeoutOrNull(ms) {
        addresses.distinctUntilChanged().first { now ->
            if (now.isEmpty()) { gone = true; false } else gone || now != tried
        }
    } != null
}
