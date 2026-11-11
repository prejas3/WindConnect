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

    /** [network] is gone: true when discovery knew it. */
    @Synchronized fun lost(network: K): Boolean = known.remove(network) != null

    companion object {
        /** What discovery depends on: the interface and its addresses, in a stable order. */
        fun summary(interfaceName: String?, addresses: Iterable<String>): String =
            "${interfaceName.orEmpty()}|${addresses.sorted().joinToString(",")}"
    }
}
