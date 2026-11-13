package io.github.prejas3.windconnect

import android.app.*
import android.content.*
import android.content.pm.ServiceInfo
import android.os.IBinder
import androidx.core.content.edit
import io.github.prejas3.windconnect.bindings.FileTransfer
import io.github.prejas3.windconnect.core.ReceivedFile
import io.github.prejas3.windconnect.core.WindCore
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

internal data class ReceiveUi(val enabled: Boolean = true, val ready: Boolean = false, val error: String = "", val transfers: List<FileTransfer> = emptyList(), val receipts: List<ReceivedFile> = emptyList())

/** CCS/1 RULE:capability is enabled only while an actual foreground-service owner holds a connection lease. */
internal object FileReceiving {
    const val ID = 120
    private const val CHANNEL = "file-receiving"
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val gate = Mutex()
    private val owners = mutableSetOf<Any>()
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
        owners.add(owner)
        if (core !== opened) {
            observation?.cancel(); core = opened
            observation = scope.launch {
                launch { opened.files.collect { rows ->
                    val incoming = rows.filter { it.incoming }.take(16)
                    val failed = incoming.firstOrNull { it.state == "failed" || it.state == "cancelled" }
                    if (failed != null) lastResult = if (failed.state == "cancelled") "Odbiór anulowany" else if (failed.failure == "no_space") "Za mało miejsca lub przekroczono limit odbioru" else "Nie udało się odebrać pliku"
                    if (incoming.any { it.state == "running" }) lastResult = ""
                    mutable.update { it.copy(transfers = incoming, error = if (failed?.state == "failed") lastResult else it.error) }
                    notice(context); ReceiveFilesService.instance?.checkIdle()
                } }
                launch { opened.receivedFiles.collect { receipts -> if (receipts.size > mutable.value.receipts.size) lastResult = ""; mutable.update { it.copy(receipts = receipts) }; notice(context) } }
                launch { opened.receiveError.collect { error -> if (error.isNotEmpty()) mutable.update { it.copy(error = error) } } }
            }
        }
        refreshLocked(context)
    }
    suspend fun removeOwner(context: Context, owner: Any) = gate.withLock {
        owners.remove(owner)
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
            mutable.update { it.copy(ready = false, error = "Nie można przygotować Pobranych. Odbiór jest wyłączony; połączenie i schowek mogą działać dalej.") }
        }
    }
    fun setEnabled(context: Context, value: Boolean) {
        context.getSharedPreferences("file-receiving", Context.MODE_PRIVATE).edit { putBoolean("enabled", value) }
        mutable.update { it.copy(enabled = value) }
        scope.launch { gate.withLock { refreshLocked(context) }; ReceiveFilesService.instance?.checkIdle() }
        if (value) ReceiveFilesService.ensure(context)
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
        manager.createNotificationChannel(NotificationChannel(CHANNEL, "Odbiór plików", NotificationManager.IMPORTANCE_LOW).apply { setShowBadge(false) })
        val open = PendingIntent.getActivity(context, ID, Intent(context, ReceiveFilesActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val done = active.sumOf { it.done.toLong() }; val size = active.sumOf { it.size.toLong() }
        val speed = rate.update(now, done, busy && done < size && active.none { it.state == "waiting" })
        val message = if (busy) {
            if (done == size) "Publikowanie w Pobranych…" else "${transferBytes(done)} z ${transferBytes(size)}" + (speed?.let { " · ${transferBytes(it)}/s" } ?: "")
        } else lastResult.ifEmpty { state.receipts.firstOrNull()?.let { "${it.folder}${it.name}" } ?: "Odbiór zakończony" }
        val notification = Notification.Builder(context, CHANNEL).setSmallIcon(R.drawable.ic_connection)
            .setContentTitle(if (busy) "Odbieranie plików" else if (lastResult.isNotEmpty()) "Odbiór zakończony" else "Plik w Pobranych").setContentText(message)
            .setContentIntent(open).setOnlyAlertOnce(true).setOngoing(busy).setVisibility(Notification.VISIBILITY_PRIVATE)
            .setPublicVersion(Notification.Builder(context, CHANNEL).setSmallIcon(R.drawable.ic_connection).setContentTitle("WindConnect").setContentText("Odblokuj, aby zobaczyć odbiór plików").setContentIntent(open).build())
            .apply { if (busy) {
                setProgress(1000, if (size == 0L) 0 else (done.coerceAtMost(size) * 1000 / size).toInt(), false)
                addAction(Notification.Action.Builder(null, "Anuluj", PendingIntent.getBroadcast(context, ID + 1, Intent(context, ReceiveFilesCancelReceiver::class.java), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)).build())
            } }.build()
        try { manager.notify(ID, notification) } catch (_: SecurityException) { }
    }
    fun foregroundNotice(context: Context): Notification {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel("file-receiver-owner", "Gotowość odbioru", NotificationManager.IMPORTANCE_LOW).apply { setShowBadge(false) })
        return Notification.Builder(context, "file-receiver-owner").setSmallIcon(R.drawable.ic_connection)
            .setContentTitle("WindConnect · odbiór plików").setContentText("Pobrane/WindConnect · podczas otwartej aplikacji lub aktywnego transferu")
            .setOnlyAlertOnce(true).setOngoing(true).setVisibility(Notification.VISIBILITY_PRIVATE).build()
    }
}

class ReceiveFilesCancelReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) { FileReceiving.cancelAll() }
}

/** Started from visible UI only; the persistent ConnectionService is already a legal owner in background. */
class ReceiveFilesService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var core: WindCore? = null
    override fun onCreate() {
        super.onCreate()
        try { startForeground(121, FileReceiving.foregroundNotice(this), ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE) }
        catch (_: RuntimeException) { FileReceiving.failure("Nie można uruchomić widocznego odbioru"); stopSelf(); return }
        instance = this
        scope.launch {
            try {
                val opened = withContext(NonCancellable) { (application as WindApplication).connections.acquire().also { core = it } }
                if (opened.pairedDevices().isEmpty()) { stopSelf(); return@launch }
                FileReceiving.addOwner(this@ReceiveFilesService, opened, this@ReceiveFilesService)
                checkIdle()
            } catch (failure: Exception) { if (failure is CancellationException) throw failure; FileReceiving.failure("Nie można przygotować odbioru plików"); stopSelf() }
        }
    }
    internal fun checkIdle() {
        scope.launch {
            if ((application as WindApplication).visibleActivities == 0 || !FileReceiving.enabled(this@ReceiveFilesService) || ConnectionService.foregroundOwner) {
                if (!FileReceiving.active()) stopSelf()
            }
        }
    }
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int) = START_NOT_STICKY
    override fun onBind(intent: Intent?): IBinder? = null
    override fun onDestroy() {
        if (instance === this) instance = null
        scope.cancel(); val opened = core; core = null
        CoroutineScope(Dispatchers.Main.immediate).launch {
            try { FileReceiving.removeOwner(this@ReceiveFilesService, this@ReceiveFilesService) }
            finally { if (opened != null) (application as WindApplication).connections.release(opened) }
        }
        stopForeground(STOP_FOREGROUND_REMOVE); super.onDestroy()
    }
    companion object {
        internal var instance: ReceiveFilesService? = null; private set
        fun ensure(context: Context) {
            val app = context.applicationContext as WindApplication
            if (!FileReceiving.enabled(context) || instance != null || ConnectionService.foregroundOwner || app.visibleActivities == 0) return
            try { context.startForegroundService(Intent(context, ReceiveFilesService::class.java)) }
            catch (_: RuntimeException) { FileReceiving.failure("Odbiór nie został uruchomiony. Otwórz WindConnect ponownie.") }
        }
    }
}
