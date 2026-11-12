package io.github.prejas3.windconnect.core

/**
 * CCS/1 RULE:only a difference from what discovery already uses is a network change (WN-103).
 * Android replays the networks it has as callbacks right after registration; without this
 * filter they restart mDNS ~1 s after start, while browsers are still asking (WN-89).
 * [baseline] is the state read before discovery starts: a change between that read and the
 * replay shows up as a difference and still counts. Keys are networks, values their summary
 * (interface and addresses, [summary]).
 */
internal class NetworkChangeFilter<K>(baseline: Map<K, String>) {
    private val known = HashMap(baseline)

    /** New link properties for [network]: true when its interface or addresses differ. */
    @Synchronized fun linkChanged(network: K, summary: String): Boolean = known.put(network, summary) != summary

    /**
     * [network] is gone from the callback: true when discovery knew it. [background] is its
     * summary when it still exists, only no longer in the foreground (WN-111): another app
     * dropped its request for mobile data, the callback can't see background networks, yet the
     * interface and its addresses are unchanged. That is no change for discovery; the network
     * stays known, so its return with the same summary isn't one either.
     */
    @Synchronized fun lost(network: K, background: String? = null): Boolean {
        if (background != null && known[network] == background) return false
        return known.remove(network) != null
    }

    companion object {
        /** What discovery depends on: the interface and its addresses, in a stable order. */
        fun summary(interfaceName: String?, addresses: Iterable<String>): String =
            "${interfaceName.orEmpty()}|${addresses.sorted().joinToString(",")}"
    }
}
