package io.github.prejas3.windconnect

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import io.github.prejas3.windconnect.core.ConnectionState
import io.github.prejas3.windconnect.core.WindCore
import kotlinx.coroutines.*

/** CCS/1 PURPOSE:visible, user-stoppable ownership of the paired LAN connection across UI/boot. */
class ConnectionService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var core: WindCore? = null
    private var reconnect: Job? = null
    private var peerName = "Sparowany komputer"
    private var current: ConnectionState = ConnectionState.Ready

    override fun onCreate() {
        super.onCreate()
        getSystemService(NotificationManager::class.java).createNotificationChannel(
            NotificationChannel(CHANNEL, "Połączenie z komputerem", NotificationManager.IMPORTANCE_LOW).apply {
                description = "Urządzenie, stan połączenia i szybkie akcje WindConnect"
                setShowBadge(false)
            },
        )
        startForeground(ID, notification(), ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
        scope.launch {
            try {
                val opened = (application as WindApplication).connections.acquire()
                core = opened
                val peers = opened.pairedDevices()
                if (peers.isEmpty() || !enabled(this@ConnectionService)) { stopSelf(); return@launch }
                peerName = peers.last().name.ifBlank { "Sparowany komputer" }
                launch {
                    opened.state.collect { state ->
                        current = state
                        if (state is ConnectionState.Connected) peerName = state.peerName
                        updateNotification()
                    }
                }
                reconnect = launch {
                    // CCS/1 RULE:no unauthorised address/key is persisted; every retry reloads trusted peers.
                    var backoff = 3_000L
                    while (isActive) {
                        delay(backoff)
                        if (opened.state.value !is ConnectionState.Connected && opened.state.value !is ConnectionState.Connecting) {
                            try {
                                val peer = opened.pairedDevices().lastOrNull()
                                if (peer == null) { stopSelf(); break }
                                opened.connectPaired(peer); backoff = 3_000
                            }
                            catch (error: Exception) {
                                if (error is CancellationException && error !is TimeoutCancellationException) throw error
                                backoff = (backoff * 2).coerceAtMost(60_000)
                            }
                        } else backoff = 3_000
                    }
                }
            } catch (error: Exception) {
                if (error is CancellationException) throw error
                preferences(this@ConnectionService).edit().putString("error", "Nie udało się uruchomić połączenia w tle. Otwórz aplikację i spróbuj ponownie.").apply()
                stopSelf()
            }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == STOP || !enabled(this)) {
            preferences(this).edit().putBoolean("enabled", false).apply()
            reconnect?.cancel()
            scope.launch { core?.disconnect(); stopSelf() }
            return START_NOT_STICKY
        }
        if (intent?.action == RETRY) scope.launch {
            val opened = core ?: return@launch
            if (opened.state.value !is ConnectionState.Connected && opened.state.value !is ConnectionState.Connecting) {
                try { opened.pairedDevices().lastOrNull()?.let { opened.connectPaired(it) } }
                catch (error: Exception) { if (error is CancellationException && error !is TimeoutCancellationException) throw error }
            }
        }
        return START_STICKY
    }

    private fun notification(): Notification {
        val connected = current is ConnectionState.Connected
        val text = when (current) {
            is ConnectionState.Connected -> "Połączono · " + if ((current as ConnectionState.Connected).route == "relay") "przekaźnik E2E" else "Wi-Fi"
            ConnectionState.Connecting -> "Łączenie…"
            else -> "Brak połączenia · ponawianie w tle"
        }
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        fun command(action: String, request: Int) = PendingIntent.getService(this, request, Intent(this, ConnectionService::class.java).setAction(action), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_connection)
            .setContentTitle(peerName).setContentText(text)
            .setContentIntent(open).setOngoing(true).setOnlyAlertOnce(true)
            .setVisibility(Notification.VISIBILITY_PRIVATE).setCategory(Notification.CATEGORY_SERVICE)
            .addAction(Notification.Action.Builder(null, "Otwórz urządzenie", open).build())
            .apply { if (!connected) addAction(Notification.Action.Builder(null, "Połącz ponownie", command(RETRY, 1)).build()) }
            .addAction(Notification.Action.Builder(null, "Zatrzymaj", command(STOP, 2)).build())
            .build()
    }

    private fun updateNotification() = getSystemService(NotificationManager::class.java).notify(ID, notification())

    override fun onBind(intent: Intent?): IBinder? = null
    override fun onDestroy() {
        scope.cancel()
        val opened = core
        core = null
        if (opened != null) CoroutineScope(Dispatchers.IO).launch { (application as WindApplication).connections.release(opened) }
        stopForeground(STOP_FOREGROUND_REMOVE)
        super.onDestroy()
    }

    companion object {
        const val CHANNEL = "connection"
        const val ID = 29
        private const val STOP = "io.github.prejas3.windconnect.STOP"
        private const val RETRY = "io.github.prejas3.windconnect.RETRY"
        fun preferences(context: Context) = context.getSharedPreferences("background_connection", Context.MODE_PRIVATE)
        fun enabled(context: Context) = preferences(context).getBoolean("enabled", false)
        fun start(context: Context, enable: Boolean = false): Boolean {
            if (enable) preferences(context).edit().putBoolean("enabled", true).apply()
            if (!enabled(context)) return false
            return try {
                context.startForegroundService(Intent(context, ConnectionService::class.java))
                preferences(context).edit().remove("error").apply()
                true
            } catch (_: IllegalStateException) {
                preferences(context).edit().putString("error", "System wstrzymał start w tle. Otwórz WindConnect, aby wznowić połączenie.").apply()
                false
            }
        }
        fun stop(context: Context) {
            preferences(context).edit().putBoolean("enabled", false).apply()
            context.startService(Intent(context, ConnectionService::class.java).setAction(STOP))
        }
    }
}

class ConnectionBootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        // CCS/1 RULE:credential-encrypted identity is opened after unlock, never on LOCKED_BOOT_COMPLETED.
        if (intent.action == Intent.ACTION_BOOT_COMPLETED || intent.action == Intent.ACTION_MY_PACKAGE_REPLACED) ConnectionService.start(context)
    }
}
