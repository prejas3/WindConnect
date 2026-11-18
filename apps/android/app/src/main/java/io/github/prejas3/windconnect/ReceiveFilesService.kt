package io.github.prejas3.windconnect

import android.app.*
import android.content.*
import android.content.pm.ServiceInfo
import android.os.IBinder
import androidx.core.content.edit
import io.github.prejas3.windconnect.bindings.FileTransfer
import io.github.prejas3.windconnect.bindings.FileAdmission
import io.github.prejas3.windconnect.core.ReceivedFile
import io.github.prejas3.windconnect.core.WindCore
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

internal data class ReceiveUi(val enabled: Boolean = true, val ready: Boolean = false, val error: String = "", val transfers: List<FileTransfer> = emptyList(), val receipts: List<ReceivedFile> = emptyList())

/** CCS/1 RULE:visible UI may offer files; native admission requires an actual FGS lease before partial writes. */
internal object FileReceiving {
    const val ID = 120
    private const val CHANNEL = "file-receiving"
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val gate = Mutex()
    private val owners = mutableSetOf<Any>()
    private val uiOwner = Any()
    private val uiGate = Mutex()
    private var uiCore: WindCore? = null
    private var application: WindApplication? = null
    internal val persistentOwner: Boolean get() = owners.any { it === ConnectionService.instance } && ConnectionService.foregroundOwner
    private fun foregroundOwner() = persistentOwner || owners.any {
        it === ReceiveFilesService.instance && ReceiveFilesService.instance?.ownsConnection == true
    }
    private val admissions = ReceiveAdmission(
        enabled = { application?.let { enabled(it) } == true },
        visible = { (application?.visibleActivities ?: 0) > 0 },
        owner = ::foregroundOwner,
        start = { ReceiveFilesService.startForAdmission(requireNotNull(application)) },
        changed = { ReceiveFilesService.instance?.checkIdle() },
    )
    internal val hasAdmissions: Boolean get() = admissions.active
    internal val hasWaitingAdmissions: Boolean get() = admissions.waiting
    internal fun ownerChanged() {
        admissions.ownerChanged()
        ReceiveFilesService.instance?.checkIdle()
        val app = application
        // CCS/1 RULE:a new attempt arriving during an old service's destruction gets a fresh owner, never the old acknowledgement.
        if (app != null && admissions.waiting && !foregroundOwner() && ReceiveFilesService.instance == null && app.visibleActivities > 0 && enabled(app)) {
            try { ReceiveFilesService.startForAdmission(app) } catch (_: RuntimeException) { admissions.startupFailed() }
        }
    }
    internal fun startupFailed() { admissions.startupFailed() }

    fun visibilityChanged(context: Context) {
        val app = context.applicationContext as WindApplication
        application = app
        admissions.ownerChanged()
        scope.launch { uiGate.withLock {
            if (app.visibleActivities > 0 && enabled(app)) {
                if (uiCore == null) {
                    var opened: WindCore? = null
                    var kept = false
                    try {
                        withContext(NonCancellable) { opened = app.connections.acquire() }
                        val acquired = requireNotNull(opened)
                        if (app.visibleActivities > 0 && enabled(app)) {
                            addOwner(app, acquired, uiOwner)
                            uiCore = acquired; kept = true
                        }
                    } catch (error: Exception) {
                        if (error is CancellationException) throw error
                        failure(str(R.string.receive_prepare_failed))
                    } finally { if (!kept && opened != null) {
                        withContext(NonCancellable) { try { removeOwner(app, uiOwner) } finally { app.connections.release(requireNotNull(opened)) } }
                    } }
                }
            } else {
                uiCore?.let { opened ->
                    uiCore = null
                    try { removeOwner(app, uiOwner) } finally { app.connections.release(opened) }
                }
            }
            ownerChanged()
        } }
    }
    private var core: WindCore? = null
    private var observation: Job? = null
    private var lastNotice = 0L
    private var lastBusy = false
    private val rate = TransferRate()
    private var lastResult = ""
    private val mutable = MutableStateFlow(ReceiveUi())
    val ui = mutable.asStateFlow()
    fun enabled(context: Context) = context.getSharedPreferences("file-receiving", Context.MODE_PRIVATE).getBoolean("enabled", true)
    fun failure(message: String) { mutable.update { it.copy(ready = false, error = message) } }
    suspend fun addOwner(context: Context, opened: WindCore, owner: Any) = gate.withLock {
        application = context.applicationContext as WindApplication
        owners.add(owner)
        if (core !== opened) {
            observation?.cancel(); core = opened
            // CCS/1 SEC:do not accept an old client's offer using a replacement client's FGS lease, or create a JNI reference cycle.
            val expected = java.lang.ref.WeakReference(opened)
            opened.setFileAdmission(object : FileAdmission {
                override fun begin(attempt: ULong, peer: String, id: String): Boolean = runBlocking {
                    withContext(Dispatchers.Main.immediate) { admissions.begin(attempt) { expected.get()?.let { it === core } == true } }
                }
                override fun end(attempt: ULong) { scope.launch { admissions.end(attempt) } }
            })
            observation = scope.launch {
                launch { opened.files.collect { rows ->
                    val incoming = rows.filter { it.incoming }.take(16)
                    val failed = incoming.firstOrNull { it.state == "failed" || it.state == "cancelled" }
                    if (failed != null) lastResult = if (failed.state == "cancelled") str(R.string.receive_cancelled) else if (failed.failure == "no_space") str(R.string.receive_no_space) else str(R.string.receive_failed)
                    if (incoming.any { it.state == "running" }) lastResult = ""
                    mutable.update { it.copy(transfers = incoming, error = if (failed?.state == "failed") lastResult else it.error) }
                    notice(context); ReceiveFilesService.instance?.checkIdle()
                } }
                launch { opened.receivedFiles.collect { receipts -> if (receipts.size > mutable.value.receipts.size) lastResult = ""; mutable.update { it.copy(receipts = receipts) }; notice(context) } }
                launch { opened.receiveError.collect { error -> if (error.isNotEmpty()) mutable.update { it.copy(error = error) } } }
            }
        }
        refreshLocked(context)
        admissions.ownerChanged()
    }
    suspend fun removeOwner(context: Context, owner: Any) = gate.withLock {
        owners.remove(owner)
        admissions.ownerChanged()
        if (owners.isEmpty()) {
            try { core?.disableFileReceiving() } finally {
                observation?.cancel(); observation = null; core = null
                mutable.update { it.copy(ready = false, transfers = emptyList()) }
            }
        }
    }
    private suspend fun refreshLocked(context: Context) {
        val allowed = enabled(context)
        mutable.update { it.copy(enabled = allowed) }
        try {
            if (allowed && owners.isNotEmpty()) { core?.enableFileReceiving(); mutable.update { it.copy(ready = true, error = "") } }
            else { core?.disableFileReceiving(); mutable.update { it.copy(ready = false) } }
        } catch (failure: Exception) {
            if (failure is CancellationException) throw failure
            mutable.update { it.copy(ready = false, error = str(R.string.receive_downloads_failed)) }
        }
    }
    fun setEnabled(context: Context, value: Boolean) {
        context.getSharedPreferences("file-receiving", Context.MODE_PRIVATE).edit { putBoolean("enabled", value) }
        mutable.update { it.copy(enabled = value) }
        scope.launch { gate.withLock { refreshLocked(context) }; ReceiveFilesService.instance?.checkIdle() }
        visibilityChanged(context)
    }
    fun cancelAll() { val opened = core ?: return; scope.launch { opened.files.value.filter { it.incoming && it.state !in terminal }.forEach { try { opened.cancelIncoming(it.peer, it.id) } catch (_: Exception) { } } } }
    suspend fun active(): Boolean = withContext(Dispatchers.IO) { core?.incomingActive() ?: false }
    suspend fun openUri(key: String) = core?.receivedUri(key)
    fun clearHistory() { val opened = core ?: return; scope.launch { opened.clearReceiveHistory() } }
    private val terminal = setOf("done", "failed", "cancelled")
    private fun notice(context: Context) {
        val state = mutable.value
        val active = state.transfers.filter { it.state !in terminal }
        val busy = active.isNotEmpty()
        val now = android.os.SystemClock.elapsedRealtime()
        if (busy && lastBusy && now - lastNotice < 1000) return
        if (!busy && !lastBusy && state.receipts.isEmpty()) return
        lastNotice = now; lastBusy = busy
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(CHANNEL, str(R.string.receive_title), NotificationManager.IMPORTANCE_LOW).apply { setShowBadge(false) })
        val open = PendingIntent.getActivity(context, ID, Intent(context, ReceiveFilesActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val done = active.sumOf { it.done.toLong() }; val size = active.sumOf { it.size.toLong() }
        val speed = rate.update(now, done, busy && done < size && active.none { it.state == "waiting" })
        val message = if (busy) {
            if (done == size) str(R.string.receive_publishing) else str(R.string.bytes_of, transferBytes(done), transferBytes(size)) + (speed?.let { " · " + str(R.string.per_second, transferBytes(it)) } ?: "")
        } else lastResult.ifEmpty { state.receipts.firstOrNull()?.let { "${it.folder}${it.name}" } ?: str(R.string.receive_finished) }
        val notification = Notification.Builder(context, CHANNEL).setSmallIcon(R.drawable.ic_connection)
            .setContentTitle(if (busy) str(R.string.receive_receiving) else if (lastResult.isNotEmpty()) str(R.string.receive_finished) else str(R.string.receive_in_downloads)).setContentText(message)
            .setContentIntent(open).setOnlyAlertOnce(true).setOngoing(busy).setVisibility(Notification.VISIBILITY_PRIVATE)
            .setPublicVersion(Notification.Builder(context, CHANNEL).setSmallIcon(R.drawable.ic_connection).setContentTitle("WindConnect").setContentText(str(R.string.receive_unlock)).setContentIntent(open).build())
            .apply { if (busy) {
                setProgress(1000, if (size == 0L) 0 else (done.coerceAtMost(size) * 1000 / size).toInt(), false)
                addAction(Notification.Action.Builder(null, str(R.string.cancel), PendingIntent.getBroadcast(context, ID + 1, Intent(context, ReceiveFilesCancelReceiver::class.java), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)).build())
            } }.build()
        try { manager.notify(ID, notification) } catch (_: SecurityException) { }
    }
    fun foregroundNotice(context: Context): Notification {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel("file-receiver-owner", str(R.string.receive_owner_channel), NotificationManager.IMPORTANCE_LOW).apply { setShowBadge(false) })
        return Notification.Builder(context, "file-receiver-owner").setSmallIcon(R.drawable.ic_connection)
            .setContentTitle(str(R.string.receive_owner_title)).setContentText(str(R.string.receive_owner_text))
            .setOnlyAlertOnce(true).setOngoing(true).setVisibility(Notification.VISIBILITY_PRIVATE).build()
    }
}

class ReceiveFilesCancelReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) { FileReceiving.cancelAll() }
}

/** CCS/1 RULE:started only for admission from visible UI; ConnectionService may already own background receiving. */
class ReceiveFilesService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    internal var ownsConnection = false; private set
    override fun onCreate() {
        super.onCreate()
        try { startForeground(121, FileReceiving.foregroundNotice(this), ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE) }
        catch (_: RuntimeException) { FileReceiving.failure(str(R.string.receive_start_failed)); FileReceiving.startupFailed(); stopSelf(); return }
        instance = this
        scope.launch {
            var opened: WindCore? = null
            try {
                withContext(NonCancellable) { opened = (application as WindApplication).connections.acquire() }
                if (!isActive || !FileReceiving.hasWaitingAdmissions || !FileReceiving.enabled(this@ReceiveFilesService)) { stopSelf(); return@launch }
                ownsConnection = true
                FileReceiving.addOwner(this@ReceiveFilesService, requireNotNull(opened), this@ReceiveFilesService)
                FileReceiving.ownerChanged()
                checkIdle()
                awaitCancellation()
            } catch (failure: Exception) {
                if (failure is CancellationException) throw failure
                FileReceiving.failure(str(R.string.receive_prepare_failed)); FileReceiving.startupFailed(); stopSelf()
            } finally { withContext(NonCancellable) {
                ownsConnection = false
                try { FileReceiving.removeOwner(this@ReceiveFilesService, this@ReceiveFilesService) }
                finally { opened?.let { (application as WindApplication).connections.release(it) } }
                FileReceiving.ownerChanged()
            } }
        }
    }
    internal fun checkIdle() {
        scope.launch {
            val idle = if (FileReceiving.hasAdmissions) false else !FileReceiving.active()
            if (FileReceiving.persistentOwner || (idle && !FileReceiving.hasAdmissions)) {
                ownsConnection = false
                stopSelf()
            }
        }
    }
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int) = START_NOT_STICKY
    override fun onBind(intent: Intent?): IBinder? = null
    override fun onDestroy() {
        if (instance === this) instance = null
        ownsConnection = false
        scope.cancel()
        FileReceiving.ownerChanged()
        stopForeground(STOP_FOREGROUND_REMOVE); super.onDestroy()
    }
    companion object {
        internal var instance: ReceiveFilesService? = null; private set
        fun startForAdmission(context: Context) {
            val app = context.applicationContext as WindApplication
            check(FileReceiving.enabled(context) && FileReceiving.hasAdmissions && app.visibleActivities > 0)
            if (instance == null) context.startForegroundService(Intent(context, ReceiveFilesService::class.java))
        }
    }
}
