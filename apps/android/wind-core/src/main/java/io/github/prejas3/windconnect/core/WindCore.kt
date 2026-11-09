package io.github.prejas3.windconnect.core

import android.content.Context
import io.github.prejas3.windconnect.bindings.CoreClient
import io.github.prejas3.windconnect.bindings.CoreConnection
import io.github.prejas3.windconnect.bindings.CoreException
import io.github.prejas3.windconnect.bindings.IdentityInfo
import io.github.prejas3.windconnect.bindings.IdentityStorage
import io.github.prejas3.windconnect.bindings.openClient
import io.github.prejas3.windconnect.identity.KeystoreIdentityStore
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
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
    data class Connected(val peerName: String, val fingerprint: String, val roundTripMicros: Long) : ConnectionState
    data class Disconnected(val reason: String) : ConnectionState
    data class Failed(val message: String) : ConnectionState
    data object Stopped : ConnectionState
}

/** CCS/1 M:WindCore | IO:coroutines + StateFlow -> UniFFI -> authenticated QUIC session */
class WindCore private constructor(private val client: CoreClient) {
    val identity: IdentityInfo = client.identity()
    val localAddress: String = client.localAddress()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val gate = Mutex()
    private val mutableState = MutableStateFlow<ConnectionState>(ConnectionState.Ready)
    val state: StateFlow<ConnectionState> = mutableState.asStateFlow()
    private var connection: CoreConnection? = null
    private var watcher: Job? = null
    private var stopped = false

    /** SEC:caller explicitly supplies the verified peer key; discovery cannot authorize it. */
    suspend fun connect(address: String, verifiedPeerKey: ByteArray) = withContext(Dispatchers.IO) {
        gate.withLock {
            check(!stopped) { "core is stopped" }
            releaseConnection()
            mutableState.value = ConnectionState.Connecting
            var opened: CoreConnection? = null
            try {
                client.pinPeer(verifiedPeerKey)
                opened = client.connect(address, verifiedPeerKey)
                val rtt = opened.ping().toLong()
                val peer = opened.peer()
                val active = opened
                connection = active
                mutableState.value = ConnectionState.Connected(peer.name, peer.fingerprint, rtt)
                watcher = scope.launch {
                    val reason = active.waitClosed()
                    gate.withLock {
                        if (connection === active) {
                            connection = null
                            active.destroy()
                            mutableState.value = ConnectionState.Disconnected(reason)
                        }
                    }
                }
            } catch (error: Exception) {
                opened?.destroy()
                if (error is CancellationException) {
                    mutableState.value = ConnectionState.Disconnected("cancelled")
                    throw error
                }
                mutableState.value = ConnectionState.Failed(error.message ?: error.javaClass.simpleName)
                throw error
            }
        }
    }

    suspend fun disconnect() = withContext(Dispatchers.IO) {
        gate.withLock {
            if (!stopped) {
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
                try {
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

    companion object {
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
                    WindCore(client).also { opened = it }
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
