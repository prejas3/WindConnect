package io.github.prejas3.windconnect.core

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.wifi.WifiManager
import kotlinx.coroutines.channels.Channel

/** CCS/1 RULE:callbacks only enqueue; includes LAN Wi-Fi without Internet validation. */
internal class NetworkObserver(context: Context) : AutoCloseable {
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private val wifi = context.applicationContext.getSystemService(WifiManager::class.java)
    private val multicast = wifi.createMulticastLock("WindConnect discovery").apply { setReferenceCounted(false) }
    val events = Channel<Unit>(Channel.CONFLATED)
    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) { events.trySend(Unit) }
        override fun onLost(network: Network) { events.trySend(Unit) }
        override fun onLinkPropertiesChanged(network: Network, properties: LinkProperties) { events.trySend(Unit) }
    }

    init {
        multicast.acquire()
        try {
            connectivity.registerNetworkCallback(NetworkRequest.Builder()
                .removeCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
                .addCapability(NetworkCapabilities.NET_CAPABILITY_NOT_RESTRICTED).build(), callback)
        } catch (error: Exception) {
            multicast.release()
            events.close()
            throw error
        }
    }

    override fun close() {
        try { connectivity.unregisterNetworkCallback(callback) } finally {
            events.close()
            if (multicast.isHeld) multicast.release()
        }
    }
}
