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
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.delay
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** [ConnectionState.Disconnected] reasons and the refusal message (WC-28). */
const val END_UNPAIRED_HERE = "unpaired-here"
const val REFUSED = "Komputer odrzuca połączenie — sparuj ponownie"

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

/**
 * One notification handed to the core (WN-42): [outcome] "sent" / "held" / "dropped" / "not-accepted"; [postTimeMs] Android's
 * postTime and [doneMs] the wall clock when the core returned; [listenerToDoneMs] from the listener's callback to then.
 */
/** WN-48: the PC's output volume, 0..100. */
data class PcVolume(val level: Int, val muted: Boolean)

data class NotifySend(val outcome: String, val postTimeMs: Long, val doneMs: Long, val listenerToDoneMs: Long, val packageName: String)

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

    private val mutablePaired = MutableStateFlow<List<io.github.prejas3.windconnect.bindings.PairedEntry>>(emptyList())
    /** Paired computers and those still owed an unpair notice (WC-28), newest state. */
    val paired: StateFlow<List<io.github.prejas3.windconnect.bindings.PairedEntry>> = mutablePaired.asStateFlow()

    /** Reads the list again (also pins it, as [pairedDevices] does). */
    suspend fun refreshPaired() = withContext(Dispatchers.IO) {
        mutablePaired.value = client.pairedList(pairedDirectory)
    }

    /** A local name for a paired computer, shown on this phone only. */
    suspend fun renamePaired(key: ByteArray, name: String) = withContext(Dispatchers.IO) {
        gate.withLock { check(!stopped); client.renamePaired(pairedDirectory, key, name) }
        refreshPaired()
    }

    /**
     * Removes the pairing on both sides (WC-28, PROTOCOL.md §12.10). Connected to that computer:
     * it is told now. Otherwise its key is revoked at once and the notice goes when the computer
     * is next seen on the network ([deliverNotices]).
     */
    suspend fun unpair(key: ByteArray) = withContext(Dispatchers.IO) {
        gate.withLock {
            check(!stopped)
            val current = connection
            if (current != null && current.peer().publicKey.contentEquals(key)) {
                watcher?.cancel(); watcher = null; connection = null
                try { current.unpair(pairedDirectory) } finally { current.destroy() }
                mutableState.value = ConnectionState.Disconnected(END_UNPAIRED_HERE)
            } else client.unpair(pairedDirectory, key)
            if (desiredPeer?.second?.contentEquals(key) == true) desiredPeer = null
        }
        refreshPaired()
        deliverNotices(network.value)
    }

    // CCS/1 RULE:BUSY or a refusal pauses automatic retries until the network changes or the user asks (WC-28).
    @Volatile private var holdOff = false
    /** Keys (hex) of computers that refused us in TLS; the service stops retrying them. */
    private val refused = java.util.Collections.synchronizedSet(mutableSetOf<String>())
    private fun keyId(key: ByteArray) = key.joinToString("") { "%02x".format(it) }
    private val noticeTried = java.util.Collections.synchronizedMap(mutableMapOf<String, ULong>())

    /** Sends owed unpair notices to computers seen in [snapshot], once per network generation. */
    private fun deliverNotices(snapshot: NetworkSnapshot) {
        scope.launch {
            val owed = mutablePaired.value.filter { it.owed }
            for (entry in owed) {
                val seen = snapshot.peers.firstOrNull { it.fingerprint == entry.fingerprint && it.addresses.isNotEmpty() } ?: continue
                if (noticeTried.put(entry.fingerprint, snapshot.generation) == snapshot.generation) continue
                for (address in seen.addresses.sortedBy { it.startsWith("[") }) {
                    val delivered = try { client.deliverNotice(pairedDirectory, address, entry.publicKey) } catch (error: Exception) {
                        if (error is CancellationException) throw error
                        false
                    }
                    if (delivered) { refreshPaired(); break }
                }
            }
        }
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
    suspend fun connectPaired(peer: io.github.prejas3.windconnect.bindings.PeerInfo, user: Boolean = true) = withContext(Dispatchers.IO) {
        if (user) { holdOff = false; refused.remove(keyId(peer.publicKey)) }
        else if (holdOff || keyId(peer.publicKey) in refused) return@withContext
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
    private val mutableFilesAccepted = MutableStateFlow(false)
    val filesAccepted = mutableFilesAccepted.asStateFlow()
    private val mutableFiles = MutableStateFlow<List<io.github.prejas3.windconnect.bindings.FileTransfer>>(emptyList())
    val files = mutableFiles.asStateFlow()
    val outbox get() = java.io.File(appContext.noBackupFilesDir, "file-outbox")
    private var filesCapabilityJob: Job? = null
    private var filesReady = false
    private var incomingStore: IncomingFiles? = null
    private var receivingReady = false
    private val mutableReceivedFiles = MutableStateFlow<List<ReceivedFile>>(emptyList())
    val receivedFiles = mutableReceivedFiles.asStateFlow()
    private val mutableReceiveError = MutableStateFlow("")
    val receiveError = mutableReceiveError.asStateFlow()

    /** Called only after the app's receiving foreground service owns a lease. */
    suspend fun enableFileReceiving() = withContext(Dispatchers.IO) {
        prepareFiles()
        gate.withLock {
            if (incomingStore == null) {
                val store = IncomingFiles(appContext)
                client.setFileDestination(store)
                incomingStore = store
                scope.launch { store.receipts.collect { mutableReceivedFiles.value = it } }
                scope.launch { store.error.collect { mutableReceiveError.value = it } }
            }
            receivingReady = true
            connection?.let { it.activateFiles(); it.setFileReceiving(true) }
        }
    }
    suspend fun disableFileReceiving() = withContext(Dispatchers.IO) {
        gate.withLock { receivingReady = false; connection?.setFileReceiving(false) }
        if (filesReady) client.fileTransfers().filter { it.incoming && it.state !in listOf("done", "failed", "cancelled") }.forEach { try { client.cancelIncomingFile(it.peer, it.id) } catch (_: Exception) { } }
    }
    suspend fun cancelIncoming(peer: String, id: String) = withContext(Dispatchers.IO) { client.cancelIncomingFile(peer, id) }
    suspend fun incomingActive(): Boolean = withContext(Dispatchers.IO) {
        if (!filesReady) false else client.fileTransfers().any { it.incoming && it.state !in listOf("done", "failed", "cancelled") }
    }
    suspend fun receivedUri(key: String): android.net.Uri? = withContext(Dispatchers.IO) { incomingStore?.receiptUri(key) }
    suspend fun clearReceiveHistory() = withContext(Dispatchers.IO) { incomingStore?.clearHistory() }

    /** CCS/1 RULE:optional file storage failures must not prevent clipboard/connection startup. */
    suspend fun prepareFiles() = withContext(Dispatchers.IO) {
        gate.withLock {
            if (!filesReady) withContext(NonCancellable) {
                val root = java.io.File(appContext.noBackupFilesDir, "file-engine")
                client.configureFiles(java.io.File(root, "inbox").absolutePath, java.io.File(root, "partials").absolutePath, outbox.absolutePath)
                filesReady = true
                scope.launch {
                    try { while (true) {
                        val rows = client.nextFileTransfers()
                        mutableFiles.value = rows
                        // A public receipt is already journalled before native Done; discard only its private copy.
                        withContext(Dispatchers.IO) { rows.filter { it.incoming && it.state in listOf("done", "failed", "cancelled") }.forEach { row ->
                            try { client.releaseIncomingFile(row.peer, row.id) } catch (_: Exception) { }
                        } }
                    } }
                    catch (failure: Exception) { if (failure is CancellationException) throw failure }
                }
                connection?.activateFiles()
            }
        }
    }

    suspend fun queueStagedFile(file: java.io.File, target: String): String = withContext(Dispatchers.IO) {
        gate.withLock {
            val active = connection ?: error("Brak połączenia z komputerem")
            check(active.peer().fingerprint == target) { "Zmieniono komputer docelowy. Rozpocznij wysyłkę ponownie." }
            check(active.filesAccepted()) { "Komputer nie przyjmuje plików" }
            active.queueFiles(listOf(file.absolutePath)).single()
        }
    }
    fun cancelFile(id: String) = client.cancelFile(id)

    private suspend fun startFiles(active: CoreConnection) {
        filesCapabilityJob?.cancel()
        if (filesReady) active.activateFiles()
        if (receivingReady) active.setFileReceiving(true)
        mutableFilesAccepted.value = active.filesAccepted()
        filesCapabilityJob = scope.launch {
            try { while (true) mutableFilesAccepted.value = active.nextFilesAccepted() }
            catch (error: Exception) { if (error is CancellationException) throw error }
        }
    }
    private fun stopFiles() {
        filesCapabilityJob?.cancel(); filesCapabilityJob = null
        mutableFilesAccepted.value = false
    }
    @Volatile private var connection: CoreConnection? = null
    private var watcher: Job? = null
    // WN-49: the battery goes to the PC only while a session is live.
    private val battery = BatteryReporter(context)
    private var batteryJob: Job? = null
    // WN-50: find my phone. One ringer per client: a ring outlives its session (PROTOCOL.md §9.5).
    private val ringer = PhoneRinger(context, settings) { (state.value as? ConnectionState.Connected)?.peerName }.also { ringer ->
        ringer.recover()
        ringer.stopRequested = { client.stopRinging() }
        client.setFindSink(ringer)
        client.setFindEnabled(settings.getBoolean(FIND_ALLOWED, true))
    }
    // WN-46: the phone's player for the PC. Only with notification-listener access and the person's setting.
    private val appContext = context.applicationContext
    private val mutableMediaAllowed = MutableStateFlow(settings.getBoolean(MEDIA_ALLOWED, true))
    /** "Pokaż multimedia na komputerze" (default on). */
    val mediaAllowed: StateFlow<Boolean> = mutableMediaAllowed.asStateFlow()
    private val mutableMediaAccess = MutableStateFlow(MediaListenerService.granted(appContext))
    /** Android's notification-listener access, which media sessions need (re-read by [refreshMedia]). */
    val mediaAccess: StateFlow<Boolean> = mutableMediaAccess.asStateFlow()
    private val mediaUpdates = MutableStateFlow<Pair<io.github.prejas3.windconnect.bindings.MediaNow?, ByteArray?>?>(null)
    private val media = PhoneMedia(appContext, { pcPlayer.token }) { now, art -> mediaUpdates.value = now to art }.also { media ->
        client.setMediaSink(media)
        client.setMediaEnabled(mutableMediaAllowed.value && mutableMediaAccess.value)
    }
    private var mediaJob: Job? = null
    // WN-47: the PC's player shown here (app card + Android's media controls), and the person's setting.
    private val mutablePcMediaAllowed = MutableStateFlow(settings.getBoolean(PC_MEDIA_ALLOWED, true))
    /** "Pokaż odtwarzacz komputera" (default on). */
    val pcMediaAllowed: StateFlow<Boolean> = mutablePcMediaAllowed.asStateFlow()
    private val pcPlayer = PcPlayer(appContext, { (state.value as? ConnectionState.Connected)?.peerName }) { action, position ->
        scope.launch {
            val live = gate.withLock { connection }
            try { live?.sendMediaCommand(action, position?.toULong()) } catch (error: Exception) { if (error is CancellationException) throw error }
        }
    }.also { player ->
        client.setPcMediaSink(player)
        client.setPcMediaEnabled(mutablePcMediaAllowed.value)
    }
    /** The PC's player now, or null. */
    val pcMedia: StateFlow<PcMedia?> = pcPlayer.state
    fun pcMediaCommand(action: String, positionMs: Long? = null) = pcPlayer.command(action, positionMs)
    suspend fun setPcMediaAllowed(value: Boolean) = withContext(Dispatchers.IO) {
        settings.edit().putBoolean(PC_MEDIA_ALLOWED, value).apply()
        mutablePcMediaAllowed.value = value
        client.setPcMediaEnabled(value)
        if (!value) pcPlayer.clear()
        val live = gate.withLock { connection }
        try { live?.setPcMediaAccepted(value) } catch (error: Exception) { if (error is CancellationException) throw error }
    }
    // WN-42: the phone's notifications for the PC. Their own consent (§9.6 rule 6): NotifyListenerService's access and this
    // setting, off by default; only notifications posted while a session runs (§9.7).
    private val mutableNotifyAllowed = MutableStateFlow(settings.getBoolean(NOTIFY_ALLOWED, false))
    /** "Powiadomienia na komputerze" (default off). */
    val notifyAllowed: StateFlow<Boolean> = mutableNotifyAllowed.asStateFlow()
    private val mutableNotifyAccess = MutableStateFlow(NotifyListenerService.granted(appContext))
    /** Android's access for [NotifyListenerService] (re-read by [refreshNotify]). */
    val notifyAccess: StateFlow<Boolean> = mutableNotifyAccess.asStateFlow()
    private val icons = AppIcons(appContext)
    private var notifyJob: Job? = null
    /** WN-44/45: what this session sent (the PC may dismiss and reply only to these) and the filter at the send point. */
    @Volatile private var forwarder: NotifyForwarder? = null
    /** WN-45: the running session's event queue (filter changes go through it, in order with the notifications). */
    @Volatile private var notifyEvents: Channel<NotificationEvent>? = null
    private fun sentNow(id: ByteArray) = forwarder?.isSent(id) == true
    private val notifyActions = object : io.github.prejas3.windconnect.bindings.NotifyActionSink {
        override fun dismiss(id: ByteArray) {
            if (sentNow(id)) NotifyListenerService.dismiss(id)
        }
        override fun reply(id: ByteArray, text: String) {
            val ok = sentNow(id) && NotifyListenerService.reply(appContext, id, text)
            scope.launch {
                val live = gate.withLock { connection }
                try { live?.notificationReplied(id, ok) } catch (error: Exception) { if (error is CancellationException) throw error }
            }
        }
    }.also { sink ->
        client.setNotifyActionSink(sink)
        client.setNotifyActionsEnabled(mutableNotifyAllowed.value && mutableNotifyAccess.value)
    }
    private val mutableNotifySends = MutableSharedFlow<NotifySend>(extraBufferCapacity = 64, onBufferOverflow = BufferOverflow.DROP_OLDEST)
    /** Each notification handed to the core and what came of it (diagnostics and the WN-42 latency proof; no content). */
    val notifySends: SharedFlow<NotifySend> = mutableNotifySends.asSharedFlow()
    /** The phone is ringing for the PC; [stopRinging] ends it (the PC sees "stopped on the phone"). */
    val ringing: StateFlow<Boolean> = ringer.ringing
    fun stopRinging() = client.stopRinging()
    private val mutableFindAllowed = MutableStateFlow(settings.getBoolean(FIND_ALLOWED, true))
    /** "Pozwól komputerowi dzwonić": off, `find/1` isn't announced and the PC's tile is disabled. */
    val findAllowed: StateFlow<Boolean> = mutableFindAllowed.asStateFlow()
    suspend fun setFindAllowed(value: Boolean) = withContext(Dispatchers.IO) {
        settings.edit().putBoolean(FIND_ALLOWED, value).apply()
        mutableFindAllowed.value = value
        client.setFindEnabled(value)
        val live = gate.withLock { connection }
        try { live?.setFindAccepted(value) } catch (error: Exception) { if (error is CancellationException) throw error }
    }
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
    private val systemClipboard = SystemClipboard(context, settings, scope, client::setClipboardEnabled, { state.value is ConnectionState.Connected },
        imageLimitChanged = { bytes -> client.setClipboardImageLimit(bytes.toULong()) },
        publishImage = { image, manual, onApplied ->
            withContext(Dispatchers.IO) {
                val current = gate.withLock { connection }
                if (current == null) "unavailable" to null else {
                    val progress = object : io.github.prejas3.windconnect.bindings.ClipboardImageProgress {
                        override fun applied() { onApplied() }
                    }
                    current.sendClipboardImage(image.png, image.thumbnail, image.width.toULong(), image.height.toULong(), false, manual, progress)
                        .let { it.status to it.peerLimit?.toLong() }
                }
            }
        }) { text, sensitive, manual ->
        withContext(Dispatchers.IO) {
            val current = gate.withLock { connection }
            if (current == null) "unavailable" else current.sendClipboard(text, sensitive, manual)
        }
    }
    val clipboardUi = systemClipboard.ui
    fun clipboardFocus(focused: Boolean) = systemClipboard.focus(focused)
    fun setClipboardEnabled(enabled: Boolean) = systemClipboard.setEnabled(enabled)
    /** WN-36: the image limit, 5 / 16 / 50 MB. */
    fun setClipboardImageLimit(mb: Int) = systemClipboard.setImageLimit(mb)
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
        // CCS/1 RULE:observer first: its baseline predates discovery, so the replay of the current
        // networks restarts nothing and a change in between still does (WN-103). shutdown() closes it.
        val observer = NetworkObserver(context)
        this.observer = observer
        client.startDiscovery()
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
                while (true) {
                    val snapshot = client.nextNetworkSnapshot()
                    mutableNetwork.value = snapshot
                    if (mutablePaired.value.any { it.owed }) deliverNotices(snapshot)
                }
            } catch (error: Exception) {
                if (error is CancellationException) throw error
                if (!stopped) mutableNetwork.value = mutableNetwork.value.copy(error = error.message)
            }
        }
        scope.launch {
            for (event in observer.events) {
                holdOff = false
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
                // CCS/1 RULE:a connection that failed before adoption may carry the computer's unpair notice (§12.10).
                if (error !is CancellationException && opened != null && honourUnpair(opened)) {
                    opened.destroy()
                    return
                }
                opened?.destroy()
                if (error is CancellationException) {
                    desiredPeer = null
                    mutableState.value = ConnectionState.Disconnected("cancelled")
                    throw error
                }
                if (error is CoreException.UntrustedPeer || error is CoreException.InvalidInput) desiredPeer = null
                // No notice reached us (or it expired): the computer refuses us in TLS (§12.10).
                if (error is CoreException.UntrustedPeer) { refused.add(keyId(verifiedPeerKey)); mutableState.value = ConnectionState.Failed(REFUSED); throw error }
                mutableState.value = ConnectionState.Failed(error.message ?: error.javaClass.simpleName)
                throw error
            }
    }

    /**
     * A connection that ended before it became the session: if the computer closed it with
     * UNPAIRED (it removed this phone, §12.10), forget the key of this connection's TLS peer and
     * stop wanting it. True when that happened. Holds [gate].
     */
    private suspend fun honourUnpair(failed: CoreConnection): Boolean {
        val end = withTimeoutOrNull(500) { failed.waitEnd() } ?: return false
        if (end != "unpaired") return false
        try { failed.forgetPeer(pairedDirectory) } catch (error: Exception) { if (error is CancellationException) throw error }
        desiredPeer = null
        mutableState.value = ConnectionState.Disconnected(end)
        scope.launch { refreshPaired() }
        return true
    }

    /** Makes [opened] the session (pinged first) and watches it close. Holds [gate]. */
    private suspend fun adopt(opened: CoreConnection) {
        val rtt = opened.ping().toLong()
        val peer = opened.peer()
        opened.activateClipboard()
        systemClipboard.sessionStarted()
        startFiles(opened)
        connection = opened
        mutableState.value = ConnectionState.Connected(peer.name, peer.fingerprint, rtt, opened.route())
        startBattery(opened)
        startMedia(opened)
        startNotify(opened)
        startCallVolume(opened)
        watcher = scope.launch {
            // CCS/1 FLOW:end code -> unpaired: forget this connection's TLS peer | busy/untrusted: hold retries (WC-28).
            val end = opened.waitEnd()
            gate.withLock {
                if (connection === opened) {
                    connection = null
                    stopFiles()
                    stopBattery()
                    stopMedia()
                    stopNotify()
                    stopCallVolume()
                    pcPlayer.clear()
                    when (end) {
                        "unpaired" -> {
                            try { opened.forgetPeer(pairedDirectory) } catch (error: Exception) { if (error is CancellationException) throw error }
                            desiredPeer = null
                        }
                        "busy" -> holdOff = true
                        "untrusted" -> { refused.add(keyId(peer.publicKey)); desiredPeer = null }
                    }
                    opened.destroy()
                    mutableState.value = ConnectionState.Disconnected(end)
                }
            }
            if (end == "unpaired") refreshPaired()
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
                if (error !is CancellationException && honourUnpair(direct)) { direct.destroy(); return }
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
                ringer.stop()
                stopMedia()
                stopNotify()
                pcPlayer.clear()
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

    /** Sends the current reading, then each change, on [opened] (one session at a time). */
    private fun startBattery(opened: CoreConnection) {
        stopBattery()
        battery.start()
        batteryJob = scope.launch {
            battery.readings.filterNotNull().collect { reading ->
                try { opened.sendBattery(reading.level.toUByte(), reading.charging) } catch (error: Exception) {
                    if (error is CancellationException) throw error
                    // The session is ending; its watcher stops this.
                }
            }
        }
    }

    private fun stopBattery() {
        batteryJob?.cancel()
        batteryJob = null
        battery.stop()
    }

    /** Follows the phone's player on [opened] while allowed and granted; sends only changes (§9.6). */
    private fun startMedia(opened: CoreConnection) {
        stopMedia()
        if (!(mutableMediaAllowed.value && mutableMediaAccess.value)) return
        mediaUpdates.value = null
        if (!media.start()) return
        mediaJob = scope.launch {
            mediaUpdates.filterNotNull().collect { (now, art) ->
                try { if (now == null) opened.mediaGone() else opened.sendMedia(now, art) } catch (error: Exception) {
                    if (error is CancellationException) throw error
                }
            }
        }
    }

    private fun stopMedia() {
        mediaJob?.cancel()
        mediaJob = null
        media.stop()
    }

    /** Re-reads access (the person may have changed it in system settings) and applies it, live too. */
    suspend fun refreshMedia() = withContext(Dispatchers.IO) {
        mutableMediaAccess.value = MediaListenerService.granted(appContext)
        val enabled = mutableMediaAllowed.value && mutableMediaAccess.value
        client.setMediaEnabled(enabled)
        gate.withLock {
            val live = connection ?: return@withLock
            try { live.setMediaControlAccepted(enabled) } catch (error: Exception) { if (error is CancellationException) throw error }
            if (enabled && mediaJob == null) startMedia(live)
            if (!enabled && mediaJob != null) {
                stopMedia()
                try { live.mediaGone() } catch (error: Exception) { if (error is CancellationException) throw error }
            }
        }
    }

    suspend fun setMediaAllowed(value: Boolean) {
        settings.edit().putBoolean(MEDIA_ALLOWED, value).apply()
        mutableMediaAllowed.value = value
        MediaListenerService.follow(appContext, value)
        refreshMedia()
    }

    /**
     * Forwards new notifications to [opened] while allowed and granted. Events queue in order (posted then removed); the core
     * applies §9.7's window. The app's label and icon are made here, off the listener's thread.
     */
    private fun startNotify(opened: CoreConnection) {
        stopNotify()
        if (!(mutableNotifyAllowed.value && mutableNotifyAccess.value)) return
        val events = Channel<NotificationEvent>(256, BufferOverflow.DROP_OLDEST)
        NotifyListenerService.forget()
        val forward = NotifyForwarder(
            allows = { NotifyApps.allows(appContext, it) },
            send = { n, packageName -> opened.sendNotification(n.copy(app = icons.label(packageName)), icons.icon(packageName)) },
            removed = { opened.notificationRemoved(it) },
            forget = { NotifyListenerService.forget(it) },
        )
        forwarder = forward
        notifyEvents = events
        NotifyListenerService.sink = { events.trySend(it) }
        notifyJob = scope.launch {
            try {
                for (event in events) {
                    try {
                        val outcome = forward.handle(event)
                        if (event is NotificationEvent.Posted && outcome != null) {
                            mutableNotifySends.tryEmit(NotifySend(outcome, event.notification.timeMs.toLong(), System.currentTimeMillis(),
                                SystemClock.elapsedRealtime() - event.posted, event.notification.packageName))
                        }
                    } catch (error: Exception) {
                        if (error is CancellationException) throw error
                        // The session is ending; its watcher stops this.
                    }
                }
            } finally { events.close() }
        }
    }

    private fun stopNotify() {
        NotifyListenerService.sink = null
        notifyEvents = null
        forwarder = null
        notifyJob?.cancel()
        notifyJob = null
    }

    /** Re-reads access (the person may have changed it in system settings) and applies it, live too. */
    suspend fun refreshNotify() = withContext(Dispatchers.IO) {
        mutableNotifyAccess.value = NotifyListenerService.granted(appContext)
        NotifyListenerService.recordActive(appContext)
        val enabled = mutableNotifyAllowed.value && mutableNotifyAccess.value
        client.setNotifyActionsEnabled(enabled)
        gate.withLock {
            val live = connection ?: return@withLock
            try { live.setNotifyActionsAccepted(enabled) } catch (error: Exception) { if (error is CancellationException) throw error }
            if (enabled && notifyJob == null) startNotify(live)
            if (!enabled && notifyJob != null) stopNotify()
        }
    }

    // WN-48: the call state for the PC (it pauses its players) and both volumes.
    private var callWatcher: CallWatcher? = null
    private var callNotifications: Job? = null
    private var phoneVolume: PhoneVolume? = null
    private val mutableVolumeAllowed = MutableStateFlow(settings.getBoolean(VOLUME_ALLOWED, true))
    /** "Głośność z komputera" (default on): the PC shows and changes STREAM_MUSIC, and this shows the PC's. */
    val volumeAllowed: StateFlow<Boolean> = mutableVolumeAllowed.asStateFlow()
    private val mutablePcVolume = MutableStateFlow<PcVolume?>(null)
    /** The PC's output volume, or null without one. */
    val pcVolume: StateFlow<PcVolume?> = mutablePcVolume.asStateFlow()
    private val volumeSink = object : io.github.prejas3.windconnect.bindings.VolumeSink {
        override fun peerLevel(level: UByte, muted: Boolean) { mutablePcVolume.value = PcVolume(level.toInt(), muted) }
        override fun set(level: UByte?, muted: Boolean?) { phoneVolume?.set(level?.toInt(), muted) }
    }.also { sink ->
        client.setVolumeSink(sink)
        client.setVolumeEnabled(mutableVolumeAllowed.value)
    }

    private fun startCallVolume(opened: CoreConnection) {
        // Not clearing the PC's volume: its first Level may already have come with the session.
        stopCallVolume(keepPeer = true)
        val watcher = CallWatcher(appContext) { state -> opened.setCallState(state.code.toUByte()) }
        callWatcher = watcher
        watcher.start()
        callNotifications = scope.launch { CallNotifications.active.collect { watcher.notifications(it.isNotEmpty()) } }
        phoneVolume = PhoneVolume(appContext) { level, muted -> opened.setVolumeLevel(level.toUByte(), muted) }.also { it.start() }
    }

    private fun stopCallVolume(keepPeer: Boolean = false) {
        callWatcher?.stop(); callWatcher = null
        callNotifications?.cancel(); callNotifications = null
        phoneVolume?.stop(); phoneVolume = null
        if (!keepPeer) mutablePcVolume.value = null
    }

    /** WN-48: the person changes the PC's volume (only what is given); the latest wins. */
    fun setPcVolume(level: Int? = null, muted: Boolean? = null) {
        val live = connection ?: return
        live.sendVolumeSet(level?.coerceIn(0, 100)?.toUByte(), muted)
    }

    suspend fun setVolumeAllowed(value: Boolean) = withContext(Dispatchers.IO) {
        settings.edit().putBoolean(VOLUME_ALLOWED, value).apply()
        mutableVolumeAllowed.value = value
        client.setVolumeEnabled(value)
        if (!value) mutablePcVolume.value = null
        val live = gate.withLock { connection }
        try { live?.setVolumeAccepted(value) } catch (error: Exception) { if (error is CancellationException) throw error }
    }

    /** WN-45: the app filter (mode, switches, the apps seen); null until first read. */
    val notifyApps: StateFlow<NotifyAppsState?> = NotifyApps.states(appContext)

    /** WN-45: an app's label for the filter's list. */
    fun appLabel(packageName: String): String = icons.label(packageName)

    /** WN-45: "Wszystkie oprócz wyłączonych" or "Tylko wybrane"; blocked apps' notifications go away on the PC. */
    fun setNotifyAppMode(mode: NotifyAppMode) {
        NotifyApps.setMode(appContext, mode)
        notifyEvents?.trySend(NotificationEvent.FilterChanged)
    }

    /** WN-45: an app on or off for the PC, in the current mode. */
    fun setNotifyApp(packageName: String, on: Boolean) {
        NotifyApps.set(appContext, packageName, on)
        notifyEvents?.trySend(NotificationEvent.FilterChanged)
    }

    suspend fun setNotifyAllowed(value: Boolean) {
        settings.edit().putBoolean(NOTIFY_ALLOWED, value).apply()
        mutableNotifyAllowed.value = value
        NotifyListenerService.follow(appContext, value)
        refreshNotify()
    }

    private suspend fun releaseConnection() {
        stopFiles()
        stopBattery()
        stopMedia()
        stopNotify()
        stopCallVolume()
        pcPlayer.clear()
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
        internal const val SETTINGS_FILE = SETTINGS
        internal const val MEDIA_ALLOWED = "media_allowed"
        internal const val NOTIFY_ALLOWED = "notify_allowed"
        private const val VOLUME_ALLOWED = "volume_allowed"
        private const val PC_MEDIA_ALLOWED = "pc_media_allowed"
        private const val RELAY_URL = "relay_url"
        private const val FIND_ALLOWED = "find_allowed"

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
                        it.systemClipboard.applyImageLimit()
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
