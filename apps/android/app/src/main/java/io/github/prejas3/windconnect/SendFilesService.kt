package io.github.prejas3.windconnect

import android.app.*
import android.content.*
import android.content.pm.ServiceInfo
import android.net.Uri
import android.os.IBinder
import io.github.prejas3.windconnect.core.ConnectionState
import io.github.prejas3.windconnect.core.WindCore
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import java.io.File
import java.util.UUID

internal data class TransferRow(val name: String, val done: Long, val size: Long, val state: String)
internal data class FileSendStatus(val busy: Boolean = false, val message: String = "", val count: Int = 0, val completed: Int = 0, val target: String = "", val done: Long = 0, val total: Long? = null, val rate: Long? = null, val rows: List<TransferRow> = emptyList(), val canRetry: Boolean = false)
internal object FileSending {
    @Volatile var debugPhase = "idle"
    fun trace(phase: String) { if (BuildConfig.DEBUG) debugPhase = phase }
    private val mutable = MutableStateFlow(FileSendStatus())
    val status = mutable.asStateFlow()
    fun update(value: FileSendStatus) { mutable.value = value }
    fun message(value: String) { mutable.update { it.copy(message = value) } }
}

/** CCS/1 RULE:user-started batch only; temporary lease, no persistent connection preference or sticky replay. */
class SendFilesService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var sending: Job? = null
    private var terminal = false
    private var lastNotice = 0L
    private var targetName = ""

    override fun onCreate() {
        super.onCreate()
        getSystemService(NotificationManager::class.java).createNotificationChannel(
            NotificationChannel(CHANNEL, str(R.string.send_channel), NotificationManager.IMPORTANCE_LOW).apply { setShowBadge(false) })
        startForeground(ID, notification(str(R.string.send_preparing), true), ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
    }

    @Suppress("DEPRECATION")
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == CANCEL) {
            FileSending.trace("cancel-request")
            if (sending?.isActive == true) FileSending.message(str(R.string.send_cancelling))
            sending?.cancel(); if (sending == null) stopSelf(); return START_NOT_STICKY
        }
        if (intent?.action != SEND) { stopSelf(); return START_NOT_STICKY }
        if (sending?.isActive == true) return START_NOT_STICKY
        val uris = if (android.os.Build.VERSION.SDK_INT >= 33) intent.getParcelableArrayListExtra(URIS, Uri::class.java)
            else intent.getParcelableArrayListExtra<Uri>(URIS)
        val target = intent.getStringExtra(TARGET)
        sending = scope.launch { send(uris.orEmpty(), target) }
        return START_NOT_STICKY
    }

    private fun status(message: String, count: Int, completed: Int, done: Long = 0, total: Long? = null, rate: Long? = null, rows: List<TransferRow> = emptyList()) {
        FileSending.update(FileSendStatus(true, message, count, completed, targetName, done, total, rate, rows))
        val now = android.os.SystemClock.elapsedRealtime()
        if (now - lastNotice >= 1_000) {
            lastNotice = now
            getSystemService(NotificationManager::class.java).notify(ID, notification(message, true))
        }
    }

    private suspend fun send(input: List<Uri>, target: String?) {
        val owner = (application as WindApplication).connections
        var opened: WindCore? = null
        var batch: File? = null
        val pending = linkedMapOf<String, File>()
        val indices = linkedMapOf<String, Int>()
        val confirmed = mutableSetOf<Int>()
        var uris = emptyList<Uri>()
        var updates: Job? = null
        var reconnect: Job? = null
        var completed = 0
        val count = input.size
        var result = FileSendStatus(false, str(R.string.send_cancelled), count, completed)
        try {
            FileRetry.selection = null
            uris = FileImport.checkedUris(input)
            require(uris.isNotEmpty()) { str(R.string.send_none) }
            status(str(R.string.send_connecting), count, completed)
            val core = owner.acquire()
            opened = core
            val peer = core.pairedDevices().firstOrNull { it.fingerprint == target }
                ?: error(str(R.string.share_not_paired))
            targetName = computerLabel(peer.name)
            core.prepareFiles()
            if ((core.state.value as? ConnectionState.Connected)?.fingerprint != target)
                withTimeout(30_000) { core.connectPaired(peer) }
            check(core.filesAccepted.value) { str(R.string.send_not_accepted) }
            reconnect = scope.launch {
                var backoff = 3_000L
                while (isActive) {
                    core.state.first { it !is ConnectionState.Connected && it !is ConnectionState.Connecting }
                    delay(backoff)
                    if (core.state.value is ConnectionState.Connected || core.state.value is ConnectionState.Connecting) continue
                    try {
                        val trusted = core.pairedDevices().firstOrNull { it.fingerprint == target } ?: break
                        core.connectPaired(trusted, user = false)
                        backoff = 3_000L
                    } catch (failure: Exception) {
                        if (failure is CancellationException && failure !is TimeoutCancellationException) throw failure
                        backoff = (backoff * 2).coerceAtMost(30_000)
                    }
                }
            }
            val root = core.outbox.canonicalFile
            withContext(Dispatchers.IO) {
                root.listFiles()?.filter { it.name.matches(Regex("batch-[0-9a-f-]{36}")) }?.forEach { removeBatch(root, it) }
            }
            val directory = File(root, "batch-${UUID.randomUUID()}")
            check(directory.mkdir()) { str(R.string.send_prepare_failed) }
            batch = directory
            var failed = false
            var stagedBytes = 0L
            var stagedFiles = 0
            val rate = TransferRate()
            fun process() {
                val rows = core.files.value.filterNot { it.incoming }.associateBy { it.id }
                val done = pending.keys.filter { rows[it]?.state == "done" }
                done.forEach { id -> pending.remove(id)?.delete(); indices[id]?.let { confirmed.add(it) }; completed++ }
                if (pending.keys.any { rows[it]?.state in listOf("failed", "cancelled") }) failed = true
                val waiting = pending.keys.any { rows[it]?.state == "waiting" }
                val batchRows = indices.keys.mapNotNull { rows[it] }
                val bytes = batchRows.sumOf { it.done.toLong() }
                val speed = rate.update(android.os.SystemClock.elapsedRealtime(), bytes, !waiting && batchRows.any { it.state == "running" })
                status(if (waiting) str(R.string.send_waiting) else plural(R.plurals.send_received_of, count, completed, count), count, completed, bytes,
                    stagedBytes.takeIf { stagedFiles == count }, speed,
                    batchRows.take(100).map { TransferRow(it.name, it.done.toLong(), it.size.toLong(), it.state) })
            }
            suspend fun awaitProgress(until: () -> Boolean) {
                process()
                while (!until() && !failed) {
                    val before = core.files.value
                    process()
                    if (until() || failed) break
                    withTimeout(120_000) { core.files.first { it != before } }
                    process()
                }
            }
            updates = scope.launch { core.files.collect { process() } }
            val used = mutableSetOf<String>()
            var staged = 0L
            for ((index, uri) in uris.withIndex()) {
                // CCS/1 PERF:pipeline at most3 outstanding copies; receiver-confirmed files are deleted immediately.
                if (pending.size >= 3) awaitProgress { pending.size < 3 }
                check(!failed) { str(R.string.send_rejected_one) }
                status(str(R.string.send_preparing_file, index + 1, count), count, completed)
                val source = withContext(Dispatchers.IO) { FileImport.describe(this@SendFilesService, uri, used) }
                FileSending.trace("staging")
                var lastStageProgress = 0L
                val (file, size) = FileImport.stage(this, source, directory, staged) { bytes ->
                    val now = android.os.SystemClock.elapsedRealtime()
                    if (now - lastStageProgress >= 100) {
                        lastStageProgress = now
                        FileSending.message(str(R.string.send_prepared_bytes, transferBytes(bytes), index + 1, count))
                    }
                }
                staged += size
                stagedBytes += size; stagedFiles++
                currentCoroutineContext().ensureActive()
                withTimeout(120_000) { core.state.first { it is ConnectionState.Connected } }
                // CCS/1 SEC:register the committed engine ID even if user cancellation races the FFI return.
                withContext(NonCancellable) {
                    val id = core.queueStagedFile(file, peer.fingerprint)
                    pending[id] = file; indices[id] = index
                }
                process()
            }
            awaitProgress { pending.isEmpty() }
            check(!failed && completed == count) { str(R.string.send_incomplete) }
            result = FileSendStatus(false, plural(R.plurals.send_sent_to, completed, completed, targetName), count, completed, targetName, stagedBytes, stagedBytes)
        } catch (error: Exception) {
            val message = when (error) {
                is TimeoutCancellationException -> str(R.string.send_timeout)
                is CancellationException -> str(R.string.send_cancelled)
                is SecurityException -> str(R.string.send_no_access)
                is IllegalArgumentException, is IllegalStateException -> error.message ?: str(R.string.send_failed)
                else -> str(R.string.send_read_failed)
            }
            result = FileSendStatus(false, message, count, completed, targetName)
        } finally {
            FileSending.trace("cleanup-start")
            // CCS/1 RULE:keep terminal publication/stopSelf inside NonCancellable on Main; returning from IO to a cancelled parent can otherwise skip them.
            withContext(NonCancellable) {
                try {
                    withContext(Dispatchers.IO) {
                        FileSending.trace("cleanup-observer")
                        updates?.cancelAndJoin()
                        FileSending.trace("cleanup-reconnect")
                        reconnect?.cancelAndJoin()
                        val core = opened
                        if (core != null) {
                            pending.keys.forEach { try { core.cancelFile(it) } catch (_: Exception) { } }
                            core.files.value.filter { !it.incoming && it.id in indices && it.state == "done" }.forEach { indices[it.id]?.let(confirmed::add) }
                            FileSending.trace("cleanup-files")
                            try { batch?.let { removeBatch(core.outbox.canonicalFile, it) } }
                            catch (_: Exception) { /* CCS/1 STATE:abandoned private batch is retried by the next batch's cleanup. */ }
                            finally { FileSending.trace("cleanup-lease"); owner.release(core) }
                        }
                    }
                } finally {
                    val remaining = FileRetry.remaining(uris, confirmed)
                    FileRetry.selection = if (remaining.isNotEmpty() && target != null) RetrySelection(remaining, target) else null
                    result = result.copy(completed = confirmed.size, canRetry = FileRetry.selection != null)
                    terminal = true
                    FileSending.trace("terminal")
                    FileSending.update(result)
                    try {
                        stopForeground(STOP_FOREGROUND_DETACH)
                        getSystemService(NotificationManager::class.java).notify(ID, notification(FileSending.status.value.message, false))
                    } finally { stopSelf() }
                }
            }
        }
    }

    private fun notification(text: String, busy: Boolean): Notification {
        val open = PendingIntent.getActivity(this, 40, Intent(this, ShareFilesActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val progress = FileSending.status.value
        val detail = if (busy) (progress.total?.let { str(R.string.bytes_of, transferBytes(progress.done), transferBytes(it)) } ?: transferBytes(progress.done)) +
            " · " + (progress.rate?.let { str(R.string.per_second, transferBytes(it)) } ?: "—") else text
        return Notification.Builder(this, CHANNEL).setSmallIcon(R.drawable.ic_connection).setColor(getColor(R.color.icon_background)).setContentTitle(str(R.string.send_channel))
            .setContentText(detail).setSubText(text).setContentIntent(open).setOnlyAlertOnce(true).setOngoing(busy).setVisibility(Notification.VISIBILITY_PRIVATE)
            .apply { if (busy) {
                val total = progress.total
                if (total == null) setProgress(0, 0, true)
                else setProgress(1000, if (total == 0L) 0 else ((progress.done.coerceAtMost(total) * 1000) / total).toInt(), false)
            } }
            .apply { if (!busy && progress.canRetry) addAction(Notification.Action.Builder(null, str(R.string.retry), open).apply { if (android.os.Build.VERSION.SDK_INT >= 31) setAuthenticationRequired(true) }.build()) }
            .apply { if (busy) addAction(Notification.Action.Builder(null, str(R.string.cancel), PendingIntent.getService(this@SendFilesService, 41,
                Intent(this@SendFilesService, SendFilesService::class.java).setAction(CANCEL), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)).build()) }
            .build()
    }
    override fun onBind(intent: Intent?): IBinder? = null
    override fun onDestroy() { scope.cancel(); if (!terminal) stopForeground(STOP_FOREGROUND_REMOVE); super.onDestroy() }

    companion object {
        private const val CHANNEL = "files"
        private const val ID = 40
        private const val SEND = "io.github.prejas3.windconnect.SEND_FILES"
        private const val CANCEL = "io.github.prejas3.windconnect.CANCEL_FILES"
        private const val URIS = "uris"
        private const val TARGET = "target"
        fun start(context: Context, uris: List<Uri>, target: String) {
            require(uris.isNotEmpty())
            val clip = ClipData.newRawUri(str(R.string.files_clip_label), uris.first())
            uris.drop(1).forEach { clip.addItem(ClipData.Item(it)) }
            context.startForegroundService(Intent(context, SendFilesService::class.java).setAction(SEND)
                .putParcelableArrayListExtra(URIS, ArrayList(uris)).putExtra(TARGET, target)
                .apply { clipData = clip; addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION) })
        }
        fun cancel(context: Context) { context.startService(Intent(context, SendFilesService::class.java).setAction(CANCEL)) }
        internal fun removeBatch(root: File, batch: File) {
            if (batch.canonicalFile.parentFile != root.canonicalFile || java.nio.file.Files.isSymbolicLink(batch.toPath())) return
            java.nio.file.Files.walkFileTree(batch.toPath(), object : java.nio.file.SimpleFileVisitor<java.nio.file.Path>() {
                override fun visitFile(file: java.nio.file.Path, attrs: java.nio.file.attribute.BasicFileAttributes): java.nio.file.FileVisitResult {
                    java.nio.file.Files.deleteIfExists(file); return java.nio.file.FileVisitResult.CONTINUE
                }
                override fun postVisitDirectory(dir: java.nio.file.Path, error: java.io.IOException?): java.nio.file.FileVisitResult {
                    if (error != null) throw error
                    java.nio.file.Files.deleteIfExists(dir); return java.nio.file.FileVisitResult.CONTINUE
                }
            })
        }
    }
}
