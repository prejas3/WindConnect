package io.github.prejas3.windconnect

import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
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
internal suspend fun awaitPcReturn(addresses: Flow<Set<String>>, tried: Set<String>, ms: Long): Boolean {
    var gone = tried.isEmpty()
    return withTimeoutOrNull(ms) {
        addresses.distinctUntilChanged().first { now ->
            if (now.isEmpty()) { gone = true; false } else gone || now != tried
        }
    } != null
}
