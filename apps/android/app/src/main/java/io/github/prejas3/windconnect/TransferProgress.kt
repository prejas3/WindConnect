package io.github.prejas3.windconnect

import android.net.Uri

/** CCS/1 PERF:one-second monotonic windows + EMA; no ticker and no invented initial speed. */
internal class TransferRate {
    private var at: Long? = null
    private var start = 0L
    private var previous = 0L
    private var value: Double? = null
    fun update(now: Long, bytes: Long, running: Boolean): Long? {
        if (!running || bytes < previous) { at = null; value = null }
        previous = bytes
        if (!running) return null
        val from = at ?: now.also { at = it; start = bytes }
        val elapsed = now - from
        if (elapsed >= 1000) {
            val next = (bytes - start).coerceAtLeast(0).toDouble() * 1000 / elapsed
            value = value?.let { it * .5 + next * .5 } ?: next
            at = now; start = bytes
        }
        return value?.toLong()
    }
}

internal data class RetrySelection(val uris: List<Uri>, val target: String)
internal object FileRetry {
    @Volatile var selection: RetrySelection? = null
    fun <T> remaining(uris: List<T>, confirmed: Set<Int>) = uris.filterIndexed { index, _ -> index !in confirmed }
}

internal fun transferBytes(bytes: Long): String {
    var number = bytes.coerceAtLeast(0).toDouble(); var unit = 0
    val units = listOf("B", "KiB", "MiB", "GiB")
    while (number >= 1024 && unit < units.lastIndex) { number /= 1024; unit++ }
    return if (unit == 0) "${number.toLong()} B" else String.format(java.util.Locale.ROOT, "%.1f %s", number, units[unit])
}
