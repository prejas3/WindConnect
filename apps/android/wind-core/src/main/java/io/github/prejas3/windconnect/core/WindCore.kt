package io.github.prejas3.windconnect.core

import android.content.Context
import android.content.SharedPreferences
import android.os.SystemClock
import io.github.prejas3.windconnect.bindings.CoreClient
import io.github.prejas3.windconnect.bindings.CoreConnection
import io.github.prejas3.windconnect.bindings.CoreException
import io.github.prejas3.windconnect.bindings.IdentityInfo
import io.github.prejas3.windconnect.bindings.IdentityStorage
import io.github.prejas3.windconnect.bindings.openClient
import io.github.prejas3.windconnect.bindings.NetworkSnapshot
import io.github.prejas3.windconnect.identity.KeystoreIdentityStore
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

sealed interface ConnectionState {
    data object Ready : ConnectionState
    data object Connecting : ConnectionState
    /** [route]: "lan" (direct) or "relay" (PROTOCOL.md §13, only when [WindCore.setRelay] turned it on). */
    data class Connected(
        val peerName: String,
        val fingerprint: String,
        val roundTripMicros: Long,
        val route: String = "lan",
    ) : ConnectionState
    data class Disconnected(val reason: String) : ConnectionState
    data class Failed(val message: String) : ConnectionState
    data object Stopped : ConnectionState
}

/** CCS/1 M:WindCore | IO:coroutines + StateFlow -> UniFFI -> authenticated QUIC session */
class WindCore private constructor(
    private val client: CoreClient,
    private val pairedDirectory: String,
    val skippedPairingRecords: ULong,
    private val settings: SharedPreferences,
    context: Context,
) {
    suspend fun pairedDevices(): List<io.github.prejas3.windconnect.bindings.PeerInfo> = withContext(Dispatchers.IO) {
        gate.withLock { check(!stopped); client.loadPaired(pairedDirectory).devices }
    }

    suspend fun pairQr(offer: io.github.prejas3.windconnect.bindings.QrOffer): io.github.prejas3.windconnect.bindings.PeerInfo = withContext(Dispatchers.IO) {
        gate.withLock {
            check(!stopped) { "core is stopped" }
            client.pairQr(offer, pairedDirectory)
        }
    }
    /**
     * Pairing by code comparison (WN-27, PROTOCOL.md §12.6) with a computer whose SAS window
     * is open (`pairingPort`). Read the code with [SasPairing.code], show it, and pass the
     * person's answer to [SasPairing.decide]; only a match accepted on both screens pairs.
     */
    suspend fun startSas(device: io.github.prejas3.windconnect.bindings.DiscoveredDevice): io.github.prejas3.windconnect.bindings.SasPairing = withContext(Dispatchers.IO) {
        gate.withLock {
            check(!stopped) { "core is stopped" }
            client.startSas(device, pairedDirectory)
        }
    }

    /** CCS/1 SEC:discovery supplies addresses only; the key must already be in the durable paired store. */
    suspend fun connectPaired(peer: io.github.prejas3.windconnect.bindings.PeerInfo) = withContext(Dispatchers.IO) {
        withTimeout(7_000) {
            gate.withLock {
                check(!stopped) { "core is stopped" }
                val trusted = client.loadPaired(pairedDirectory).devices.firstOrNull { it.publicKey.contentEquals(peer.publicKey) }
                    ?: error("device is not paired")
                refreshDiscovery()
                client.networkChanged()
                val snapshot = network.first { snapshot -> snapshot.peers.any { it.fingerprint == trusted.fingerprint && it.addresses.isNotEmpty() } }
                val addresses = snapshot.peers.first { it.fingerprint == trusted.fingerprint }.addresses.sortedBy { it.startsWith("[") }
                for (address in addresses) {
                    val connected = withTimeoutOrNull(2_000) {
                        try { connectLocked(address, trusted.publicKey); true }
                        catch (error: Exception) { if (error is CancellationException) throw error; false }
                    }
                    if (connected == true) return@withLock
                }
                error("paired computer is not reachable")
            }
        }
    }
    val identity: IdentityInfo = client.identity()
    val localAddress: String = client.localAddress()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val gate = Mutex()
    private val mutableState = MutableStateFlow<ConnectionState>(ConnectionState.Ready)
    val state: StateFlow<ConnectionState> = mutableState.asStateFlow()
    private var connection: CoreConnection? = null
    private var watcher: Job? = null
    private var stopped = false
    private var observer: NetworkObserver? = null
    private var desiredPeer: Pair<String, ByteArray>? = null
    private var networkRetry: Job? = null
    private val mutableNetwork = MutableStateFlow(NetworkSnapshot(0uL, emptyList(), null))
    val network: StateFlow<NetworkSnapshot> = mutableNetwork.asStateFlow()
    private val mutableRelay = MutableStateFlow<String?>(null)
    /** The relay base URL, or null: off, the default (decision D3). */
    val relayUrl: StateFlow<String?> = mutableRelay.asStateFlow()
    private var lanReturn: Job? = null
    @Volatile private var multicast: MulticastPolicy? = null
    private var multicastTimer: Job? = null
    private val systemClipboard = SystemClipboard(context, settings, scope, client::setClipboardEnabled, { state.value is ConnectionState.Connected }) { text, sensitive, manual ->
        withContext(Dispatchers.IO) {
            val current = gate.withLock { connection }
            if (current == null) "unavailable" else current.sendClipboard(text, sensitive, manual)
        }
    }
    val clipboardUi = systemClipboard.ui
    fun clipboardFocus(focused: Boolean) = systemClipboard.focus(focused)
    fun setClipboardEnabled(enabled: Boolean) = systemClipboard.setEnabled(enabled)
    suspend fun sendCurrentClipboard(): String = systemClipboard.send(true)

    /**
     * Turns the E2E relay on (`wss://…`, PROTOCOL.md §13) or off (null) and remembers it. Off by
     * default. Takes effect from the next connection; the relay only ever sees encrypted QUIC.
     * @throws CoreException.InvalidInput for a URL the core refuses.
     */
    fun setRelay(url: String?) {
        val value = url?.trim()?.takeIf { it.isNotEmpty() }
        client.setRelay(value)
        settings.edit().apply { if (value == null) remove(RELAY_URL) else putString(RELAY_URL, value) }.apply()
        mutableRelay.value = value
    }

    /** A screen that needs discovery opens (SAS pairing, device list): multicast stays on until
     * the matching `false`. Calls are counted. */
    fun discoveryScreen(open: Boolean) = withMulticast { policy, now -> policy.screen(now, open) }

    /** Look for devices again for a short while (e.g. pull to refresh), even without a network change. */
    fun refreshDiscovery() = withMulticast { policy, now -> policy.wake(now) }

    // CCS/1 RULE:every policy change reschedules its one timer; all under the policy's monitor.
    private fun withMulticast(change: (MulticastPolicy, Long) -> Unit) {
        val policy = multicast ?: return
        synchronized(policy) {
            change(policy, SystemClock.elapsedRealtime())
            multicastTimer?.cancel()
            val wait = policy.nextCheck(SystemClock.elapsedRealtime()) ?: return
            multicastTimer = scope.launch {
                delay(wait)
                withMulticast { p, now -> p.tick(now) }
            }
        }
    }

    private suspend fun observeNetwork(context: Context) {
        client.startDiscovery()
        val observer = NetworkObserver(context)
        this.observer = observer
        multicast = MulticastPolicy(observer.multicast)
        refreshDiscovery()
        scope.launch {
            // CCS/1 RULE:an active session needs no multicast; a lost one looks again (WN-83).
            state.collect { value ->
                systemClipboard.connectionChanged()
                withMulticast { policy, now -> policy.connected(now, value is ConnectionState.Connected) }
            }
        }
        scope.launch {
            try {
                while (true) { mutableNetwork.value = client.nextNetworkSnapshot() }
            } catch (error: Exception) {
                if (error is CancellationException) throw error
                if (!stopped) mutableNetwork.value = mutableNetwork.value.copy(error = error.message)
            }
        }
        scope.launch {
            for (event in observer.events) {
                try {
                    refreshDiscovery()
                    // CCS/1 RULE:outside gate; wakes a connect currently holding gate immediately.
                    client.networkChanged()
                    val now = mutableState.value
                    // CCS/1 FLOW:network change on the relay -> LAN address tried in the background -> swap (WN-60)
                    if (now is ConnectionState.Connected && now.route == "relay" && lanReturn?.isActive != true) {
                        lanReturn = scope.launch { returnToLan() }
                    }
                    if (mutableState.value !is ConnectionState.Connecting &&
                        mutableState.value !is ConnectionState.Connected && networkRetry?.isActive != true) {
                        networkRetry = scope.launch {
                            gate.withLock {
                                val peer = if (stopped) null else desiredPeer
                                if (peer != null) {
                                    try { connectLocked(peer.first, peer.second) } catch (error: Exception) {
                                        if (error is CancellationException) throw error
                                    }
                                }
                            }
                        }
                    }
                } catch (error: Exception) {
                    if (error is CancellationException) throw error
                    mutableNetwork.value = mutableNetwork.value.copy(error = error.message)
                }
            }
        }
    }

    /** SEC:caller explicitly supplies the verified peer key; discovery cannot authorize it. */
    suspend fun connect(address: String, verifiedPeerKey: ByteArray) = withContext(Dispatchers.IO) {
        gate.withLock { connectLocked(address, verifiedPeerKey) }
    }

    private suspend fun connectLocked(address: String, verifiedPeerKey: ByteArray) {
            check(!stopped) { "core is stopped" }
            desiredPeer = address to verifiedPeerKey.copyOf()
            releaseConnection()
            mutableState.value = ConnectionState.Connecting
            var opened: CoreConnection? = null
            try {
                client.pinPeer(verifiedPeerKey)
                opened = client.connect(address, verifiedPeerKey)
                adopt(opened)
            } catch (error: Exception) {
                opened?.destroy()
                if (error is CancellationException) {
                    desiredPeer = null
                    mutableState.value = ConnectionState.Disconnected("cancelled")
                    throw error
                }
                if (error is CoreException.UntrustedPeer || error is CoreException.InvalidInput) desiredPeer = null
                mutableState.value = ConnectionState.Failed(error.message ?: error.javaClass.simpleName)
                throw error
            }
    }

    /** Makes [opened] the session (pinged first) and watches it close. Holds [gate]. */
    private suspend fun adopt(opened: CoreConnection) {
        val rtt = opened.ping().toLong()
        val peer = opened.peer()
        opened.activateClipboard()
        systemClipboard.sessionStarted()
        connection = opened
        mutableState.value = ConnectionState.Connected(peer.name, peer.fingerprint, rtt, opened.route())
        watcher = scope.launch {
            val reason = opened.waitClosed()
            gate.withLock {
                if (connection === opened) {
                    connection = null
                    opened.destroy()
                    mutableState.value = ConnectionState.Disconnected(reason)
                }
            }
        }
    }

    /** On the relay after a network change: the peer's address, if it answers, replaces the relay. */
    private suspend fun returnToLan() {
        val peer = desiredPeer ?: return
        val direct = try { client.connectDirect(peer.first, peer.second) } catch (error: Exception) {
            if (error is CancellationException) throw error
            return // still away from home: keep the relay
        }
        gate.withLock {
            val now = mutableState.value
            if (stopped || desiredPeer !== peer || now !is ConnectionState.Connected || now.route != "relay") {
                direct.destroy()
                return
            }
            releaseConnection()
            try { adopt(direct) } catch (error: Exception) {
                direct.destroy()
                if (error is CancellationException) throw error
                mutableState.value = ConnectionState.Disconnected(error.message ?: "LAN return failed")
            }
        }
    }

    suspend fun disconnect() = withContext(Dispatchers.IO) {
        gate.withLock {
            if (!stopped) {
                desiredPeer = null
                networkRetry?.cancel()
                lanReturn?.cancel()
                releaseConnection()
                mutableState.value = ConnectionState.Disconnected("disconnected locally")
            }
        }
    }

    /** RULE:call on owner shutdown; releases native handles even if the caller is cancelled. */
    suspend fun shutdown() = withContext(NonCancellable + Dispatchers.IO) {
        gate.withLock {
            if (!stopped) {
                stopped = true
                systemClipboard.close()
                desiredPeer = null
                networkRetry?.cancel()
                lanReturn?.cancel()
                try {
                    multicast?.let { policy -> synchronized(policy) { multicastTimer?.cancel(); policy.close() } }
                    observer?.close()
                    observer = null
                    releaseConnection()
                    client.shutdown()
                } finally {
                    client.destroy()
                    mutableState.value = ConnectionState.Stopped
                    scope.cancel()
                }
            }
        }
    }

    private suspend fun releaseConnection() {
        watcher?.cancel()
        watcher = null
        val previous = connection
        connection = null
        if (previous != null) {
            try { previous.disconnect() } finally { previous.destroy() }
        }
    }

    /** The saved relay, if any; one the core now refuses is forgotten (off). */
    private fun restoreRelay() {
        val saved = settings.getString(RELAY_URL, null) ?: return
        try { setRelay(saved) } catch (error: CoreException.InvalidInput) { setRelay(null) }
    }

    companion object {
        private const val SETTINGS = "windconnect.core"
        private const val RELAY_URL = "relay_url"

        suspend fun open(context: Context, name: String, bindAddress: String = "0.0.0.0:0"): WindCore {
            var native: CoreClient? = null
            var opened: WindCore? = null
            try {
                return withContext(Dispatchers.IO) {
                    val store = KeystoreIdentityStore(context.applicationContext)
                    val storage = object : IdentityStorage {
                        override fun load(): ByteArray? = try {
                            store.load()
                        } catch (error: Exception) {
                            throw CoreException.Storage(error.message ?: "identity load failed")
                        }

                        override fun storeNew(pem: ByteArray): Boolean = try {
                            store.storeNew(pem)
                        } catch (error: Exception) {
                            throw CoreException.Storage(error.message ?: "identity store failed")
                        } finally {
                            pem.fill(0)
                        }
                    }
                    val client = openClient(storage, name, bindAddress)
                    native = client
                    val pairedDirectory = context.noBackupFilesDir.absolutePath
                    val paired = client.loadPaired(pairedDirectory)
                    val settings = context.applicationContext.getSharedPreferences(SETTINGS, Context.MODE_PRIVATE)
                    WindCore(client, pairedDirectory, paired.skippedRecords, settings, context.applicationContext).also {
                        opened = it
                        client.setClipboardSink(it.systemClipboard)
                        client.setClipboardEnabled(it.clipboardUi.value.enabled)
                        it.restoreRelay()
                        it.observeNetwork(context.applicationContext)
                    }
                }
            } catch (error: Throwable) {
                // CCS/1 RULE:withContext prompt cancellation must not lose a returned native owner.
                withContext(NonCancellable + Dispatchers.IO) {
                    val owner = opened
                    if (owner != null) owner.shutdown() else native?.let { client ->
                        try { client.shutdown() } finally { client.destroy() }
                    }
                }
                throw error
            }
        }
    }
}
