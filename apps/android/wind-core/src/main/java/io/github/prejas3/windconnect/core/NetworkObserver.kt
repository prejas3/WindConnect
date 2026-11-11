package io.github.prejas3.windconnect.core

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.wifi.WifiManager
import kotlinx.coroutines.channels.Channel

/** CCS/1 RULE:callbacks only enqueue; includes LAN Wi-Fi without Internet validation.
 * The multicast lock is not taken here: [MulticastPolicy] switches [multicast] (WN-83). */
internal class NetworkObserver(context: Context) : AutoCloseable {
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private val wifi = context.applicationContext.getSystemService(WifiManager::class.java)
    private val lock = wifi.createMulticastLock("WindConnect discovery").apply { setReferenceCounted(false) }
    val multicast = object : MulticastSwitch {
        override val isHeld: Boolean get() = lock.isHeld
        override fun acquire() = lock.acquire()
        override fun release() = lock.release()
    }
    val events = Channel<Unit>(Channel.CONFLATED)
    // CCS/1 RULE:create before discovery starts; the baseline is read before registering (WN-103).
    private val changes = NetworkChangeFilter(current())
    private val callback = object : ConnectivityManager.NetworkCallback() {
        // From API 26 onAvailable is always followed by onLinkPropertiesChanged, which decides.
        override fun onLost(network: Network) { if (changes.lost(network)) events.trySend(Unit) }
        override fun onLinkPropertiesChanged(network: Network, properties: LinkProperties) {
            if (changes.linkChanged(network, summary(properties))) events.trySend(Unit)
        }
    }

    /** The networks the callback below would report now (same capabilities as its request). */
    @Suppress("DEPRECATION") // allNetworks: the one synchronous list; the callback replays it later.
    private fun current(): Map<Network, String> = connectivity.allNetworks.mapNotNull { network ->
        val capabilities = connectivity.getNetworkCapabilities(network) ?: return@mapNotNull null
        val properties = connectivity.getLinkProperties(network) ?: return@mapNotNull null
        if (!capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_RESTRICTED) ||
            !capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_TRUSTED)) return@mapNotNull null
        network to summary(properties)
    }.toMap()

    private fun summary(properties: LinkProperties) =
        NetworkChangeFilter.summary(properties.interfaceName, properties.linkAddresses.map { it.toString() })

    init {
        try {
            connectivity.registerNetworkCallback(NetworkRequest.Builder()
                .removeCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
                .addCapability(NetworkCapabilities.NET_CAPABILITY_NOT_RESTRICTED).build(), callback)
        } catch (error: Exception) {
            events.close()
            throw error
        }
    }

    override fun close() {
        try { connectivity.unregisterNetworkCallback(callback) } finally {
            events.close()
            if (lock.isHeld) lock.release()
        }
    }
}
