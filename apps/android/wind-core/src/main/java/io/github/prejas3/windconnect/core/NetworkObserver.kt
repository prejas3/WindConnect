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
    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) { events.trySend(Unit) }
        override fun onLost(network: Network) { events.trySend(Unit) }
        override fun onLinkPropertiesChanged(network: Network, properties: LinkProperties) { events.trySend(Unit) }
    }

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
