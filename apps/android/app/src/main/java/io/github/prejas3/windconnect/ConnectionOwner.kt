package io.github.prejas3.windconnect

import android.app.Application
import io.github.prejas3.windconnect.core.WindCore
import io.github.prejas3.windconnect.core.ConnectionState
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** CCS/1 RULE:Activity and service lease one native client; only the final release shuts it down. */
class WindApplication : Application() {
    val connections = ConnectionOwner(this)
    override fun onCreate() {
        super.onCreate()
        LauncherShortcuts.publish(this)
        WidgetUpdates.start(this)
    }
}

class ConnectionOwner(private val application: Application) {
    private val gate = Mutex()
    @Volatile private var core: WindCore? = null
    @Volatile private var focused = false
    fun clipboardFocus(value: Boolean) { focused = value; core?.clipboardFocus(value) }
    private var leases = 0
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var observation: Job? = null
    private val mutableState = MutableStateFlow<ConnectionState>(ConnectionState.Stopped)
    // CCS/1 RULE:QS observes existing leases only; opening the panel never starts a connection.
    val state = mutableState.asStateFlow()
    // WN-50: the ring and its setting, for the Activity's stop bar and Settings (no lease of their own).
    private val mutableRinging = MutableStateFlow(false)
    val ringing = mutableRinging.asStateFlow()
    private val mutableFindAllowed = MutableStateFlow(true)
    val findAllowed = mutableFindAllowed.asStateFlow()
    fun stopRinging() { core?.stopRinging() }
    // WN-46: the phone's player on the PC.
    private val mutableMediaAllowed = MutableStateFlow(true)
    val mediaAllowed = mutableMediaAllowed.asStateFlow()
    private val mutableMediaAccess = MutableStateFlow(false)
    val mediaAccess = mutableMediaAccess.asStateFlow()
    fun setMediaAllowed(value: Boolean) { val opened = core ?: return; scope.launch { opened.setMediaAllowed(value) } }
    // WN-47: the PC's player on the phone.
    private val mutablePcMedia = MutableStateFlow<io.github.prejas3.windconnect.core.PcMedia?>(null)
    val pcMedia = mutablePcMedia.asStateFlow()
    private val mutablePcMediaAllowed = MutableStateFlow(true)
    val pcMediaAllowed = mutablePcMediaAllowed.asStateFlow()
    fun pcMediaCommand(action: String, positionMs: Long? = null) { core?.pcMediaCommand(action, positionMs) }
    fun setPcMediaAllowed(value: Boolean) { val opened = core ?: return; scope.launch { opened.setPcMediaAllowed(value) } }
    /** Access is granted in system settings: re-read it when the app comes back. */
    fun refreshMedia() { val opened = core ?: return; scope.launch { opened.refreshMedia() } }
    // WN-42: the phone's notifications on the PC (own consent, default off).
    private val mutableNotifyAllowed = MutableStateFlow(false)
    val notifyAllowed = mutableNotifyAllowed.asStateFlow()
    private val mutableNotifyAccess = MutableStateFlow(false)
    val notifyAccess = mutableNotifyAccess.asStateFlow()
    fun setNotifyAllowed(value: Boolean) { val opened = core ?: return; scope.launch { opened.setNotifyAllowed(value) } }
    fun refreshNotify() { val opened = core ?: return; scope.launch { opened.refreshNotify() } }
    // WN-45: which apps go to the PC.
    private val mutableNotifyApps = MutableStateFlow<io.github.prejas3.windconnect.core.NotifyAppsState?>(null)
    val notifyApps = mutableNotifyApps.asStateFlow()
    fun setNotifyAppMode(mode: io.github.prejas3.windconnect.core.NotifyAppMode) { core?.setNotifyAppMode(mode) }
    fun setNotifyApp(packageName: String, on: Boolean) { core?.setNotifyApp(packageName, on) }
    fun appLabel(packageName: String): String = core?.appLabel(packageName) ?: packageName
    fun setFindAllowed(value: Boolean) { val opened = core ?: return; scope.launch { opened.setFindAllowed(value) } }

    suspend fun acquire(): WindCore = gate.withLock {
        val opened = core ?: WindCore.open(application, android.os.Build.MODEL.take(64)).also { opened ->
            core = opened
            mutableState.value = opened.state.value
            observation = scope.launch {
                launch { opened.ringing.collect { mutableRinging.value = it } }
                launch { opened.findAllowed.collect { mutableFindAllowed.value = it } }
                launch { opened.mediaAllowed.collect { mutableMediaAllowed.value = it } }
                launch { opened.mediaAccess.collect { mutableMediaAccess.value = it } }
                launch { opened.pcMedia.collect { mutablePcMedia.value = it } }
                launch { opened.pcMediaAllowed.collect { mutablePcMediaAllowed.value = it } }
                launch { opened.notifyAllowed.collect { mutableNotifyAllowed.value = it } }
                launch { opened.notifyAccess.collect { mutableNotifyAccess.value = it } }
                launch { opened.notifyApps.collect { mutableNotifyApps.value = it } }
                opened.state.collect { mutableState.value = it }
            }
        }
        opened.clipboardFocus(focused)
        leases++
        opened
    }

    suspend fun release(opened: WindCore) = gate.withLock {
        check(core === opened && leases > 0)
        leases--
        if (leases == 0) {
            observation?.cancelAndJoin()
            observation = null
            mutableState.value = ConnectionState.Stopped
            mutableRinging.value = false
            mutablePcMedia.value = null
            core = null
            opened.shutdown()
        }
    }
}
