package io.github.prejas3.windconnect.core

/** CCS/1 RULE:each adopted session discards pre-session clips, including after process death; no payload persists.
 * EXT:ClipDescription.timestamp uses System.currentTimeMillis(), not elapsedRealtime. */
internal class ClipboardFreshness {
    @Volatile private var baseline = Long.MAX_VALUE
    fun sessionStarted(now: Long) { baseline = now }
    fun allows(timestamp: Long): Boolean = timestamp > baseline
}
