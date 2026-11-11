package io.github.prejas3.windconnect.core

/** What [MulticastPolicy] switches: Android's `WifiManager.MulticastLock` in the app. */
internal interface MulticastSwitch {
    val isHeld: Boolean
    fun acquire()
    fun release()
}

/**
 * CCS/1 M:MulticastPolicy | IO:connection, network and UI events -> MulticastLock (WN-83)
 * RULE:held only while discovery is needed: a SAS window or discovery screen open, or no
 * session and less than [window] ms since start, a network change, a lost session or [wake].
 * DEC:an active session needs no multicast: QUIC migration (WN-16, WN-84) brings it back.
 * DEC:other PCs during a session (WN-90): the broadcast fallback (WN-15), no lock. The phone's
 * own query every 2 s (sending needs no lock) gets each PC's unicast answer, so another PC is
 * back in `network` within 5 s of a network change. PCs' broadcast queries reaching the phone
 * are a bonus, not the basis: without the lock a Wi-Fi chip may filter them. No mDNS window
 * during a session: with a paired PC simply off it would hold the lock 30 s per network change.
 * Times are monotonic milliseconds from the caller; it rechecks at [nextCheck].
 */
internal class MulticastPolicy(
    private val lock: MulticastSwitch,
    private val window: Long = DEFAULT_WINDOW_MILLIS,
) {
    private var connected = false
    private var screens = 0
    private var until = Long.MIN_VALUE
    private var closed = false

    /** Start, a network change or a request to look again: look for [window] ms. */
    @Synchronized fun wake(now: Long) {
        until = maxOf(until, now + window)
        apply(now)
    }

    /** A session is up (`true`) or gone (`false`, which looks again for [window] ms). */
    @Synchronized fun connected(now: Long, value: Boolean) {
        if (connected == value) return
        connected = value
        if (!value) until = maxOf(until, now + window)
        apply(now)
    }

    /** A screen that needs discovery (SAS pairing, device list) opens or closes; counted. */
    @Synchronized fun screen(now: Long, open: Boolean) {
        screens = if (open) screens + 1 else maxOf(0, screens - 1)
        apply(now)
    }

    /** Applies the window's end; call at [nextCheck]. */
    @Synchronized fun tick(now: Long) = apply(now)

    /** Milliseconds until the hold may end by itself, or `null` when nothing is timed. */
    @Synchronized fun nextCheck(now: Long): Long? =
        if (lock.isHeld && screens == 0 && !connected) maxOf(0, until - now) else null

    @Synchronized fun close() {
        closed = true
        if (lock.isHeld) lock.release()
    }

    private fun apply(now: Long) {
        val wanted = !closed && (screens > 0 || (!connected && now < until))
        if (wanted && !lock.isHeld) lock.acquire()
        if (!wanted && lock.isHeld) lock.release()
    }

    companion object {
        /** mDNS answers in about a second; browsers re-query at 1, 2, 4, 8 and 16 s. */
        const val DEFAULT_WINDOW_MILLIS = 30_000L
    }
}
