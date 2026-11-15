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
    var visibleActivities = 0; private set
    override fun onCreate() {
        super.onCreate()
        LauncherShortcuts.publish(this)
        WidgetUpdates.start(this)
        registerActivityLifecycleCallbacks(object : ActivityLifecycleCallbacks {
            override fun onActivityStarted(activity: android.app.Activity) { visibleActivities++; FileReceiving.visibilityChanged(activity) }
            override fun onActivityStopped(activity: android.app.Activity) { visibleActivities = (visibleActivities - 1).coerceAtLeast(0); FileReceiving.visibilityChanged(activity) }
            override fun onActivityCreated(activity: android.app.Activity, state: android.os.Bundle?) = Unit
            override fun onActivityResumed(activity: android.app.Activity) = Unit
            override fun onActivityPaused(activity: android.app.Activity) = Unit
            override fun onActivitySaveInstanceState(activity: android.app.Activity, state: android.os.Bundle) = Unit
            override fun onActivityDestroyed(activity: android.app.Activity) = Unit
        })
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
    private val mutableTouchpadAccepted = MutableStateFlow(false)
    val touchpadAccepted = mutableTouchpadAccepted.asStateFlow()
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
    // WN-48: the PC's volume, and whether the PC may show/change STREAM_MUSIC (and this shows the PC's).
    private val mutablePcVolume = MutableStateFlow<io.github.prejas3.windconnect.core.PcVolume?>(null)
    val pcVolume = mutablePcVolume.asStateFlow()
    private val mutableVolumeAllowed = MutableStateFlow(true)
    val volumeAllowed = mutableVolumeAllowed.asStateFlow()
    fun setPcVolume(level: Int? = null, muted: Boolean? = null) { core?.setPcVolume(level, muted) }
    fun setVolumeAllowed(value: Boolean) { val opened = core ?: return; scope.launch { opened.setVolumeAllowed(value) } }
    fun setFindAllowed(value: Boolean) { val opened = core ?: return; scope.launch { opened.setFindAllowed(value) } }
    // WN-55: SMS read on the PC (own consent, default off; READ_SMS asked when turned on).
    private val mutableSmsAllowed = MutableStateFlow(false)
    val smsAllowed = mutableSmsAllowed.asStateFlow()
    private val mutableSmsAccess = MutableStateFlow(false)
    val smsAccess = mutableSmsAccess.asStateFlow()
    private val mutableSmsContacts = MutableStateFlow(false)
    val smsContacts = mutableSmsContacts.asStateFlow()
    fun setSmsAllowed(value: Boolean) { val opened = core ?: return; scope.launch { opened.setSmsAllowed(value) } }
    fun refreshSms() { val opened = core ?: return; scope.launch { opened.refreshSms() } }
    // WN-56: sending from the PC (own setting, default off; SEND_SMS asked when turned on).
    private val mutableSmsSendAllowed = MutableStateFlow(false)
    val smsSendAllowed = mutableSmsSendAllowed.asStateFlow()
    private val mutableSmsSendAccess = MutableStateFlow(false)
    val smsSendAccess = mutableSmsSendAccess.asStateFlow()
    fun setSmsSendAllowed(value: Boolean) { val opened = core ?: return; scope.launch { opened.setSmsSendAllowed(value) } }
    // WN-58: photos in the PC's panel (own consent, default off; photo access asked when turned on).
    private val mutableGalleryAllowed = MutableStateFlow(false)
    val galleryAllowed = mutableGalleryAllowed.asStateFlow()
    private val mutableGalleryAccess = MutableStateFlow(false)
    val galleryAccess = mutableGalleryAccess.asStateFlow()
    fun setGalleryAllowed(value: Boolean) { val opened = core ?: return; scope.launch { opened.setGalleryAllowed(value) } }
    fun refreshGallery() { val opened = core ?: return; scope.launch { opened.refreshGallery() } }
    // WN-100: actions on the PC, and links from it.
    private val mutableOpenLinks = MutableStateFlow(true)
    val openLinks = mutableOpenLinks.asStateFlow()
    fun setOpenLinks(value: Boolean) { core?.setOpenLinks(value) }
    suspend fun runPcAction(id: String, url: String? = null): String = core?.runPcAction(id, url) ?: "unavailable"
    suspend fun pcActions(): List<io.github.prejas3.windconnect.bindings.RemoteAction> = core?.pcActions() ?: emptyList()
    // WN-57: calls on the PC (own setting, default off; phone and call-log permissions asked when turned on).
    private val mutableCallsAllowed = MutableStateFlow(false)
    val callsAllowed = mutableCallsAllowed.asStateFlow()
    private val mutableCallsAccess = MutableStateFlow(false)
    val callsAccess = mutableCallsAccess.asStateFlow()
    private val mutableCallsReject = MutableStateFlow(false)
    val callsReject = mutableCallsReject.asStateFlow()
    fun setCallsAllowed(value: Boolean) { val opened = core ?: return; scope.launch { opened.setCallsAllowed(value) } }
    fun refreshCalls() { val opened = core ?: return; scope.launch { opened.refreshCalls() } }

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
                launch { opened.touchpadAccepted.collect { mutableTouchpadAccepted.value = it } }
                launch { opened.notifyAllowed.collect { mutableNotifyAllowed.value = it } }
                launch { opened.notifyAccess.collect { mutableNotifyAccess.value = it } }
                launch { opened.notifyApps.collect { mutableNotifyApps.value = it } }
                launch { opened.pcVolume.collect { mutablePcVolume.value = it } }
                launch { opened.volumeAllowed.collect { mutableVolumeAllowed.value = it } }
                launch { opened.smsAllowed.collect { mutableSmsAllowed.value = it } }
                launch { opened.smsAccess.collect { mutableSmsAccess.value = it } }
                launch { opened.smsContacts.collect { mutableSmsContacts.value = it } }
                launch { opened.smsSendAllowed.collect { mutableSmsSendAllowed.value = it } }
                launch { opened.smsSendAccess.collect { mutableSmsSendAccess.value = it } }
                launch { opened.galleryAllowed.collect { mutableGalleryAllowed.value = it } }
                launch { opened.galleryAccess.collect { mutableGalleryAccess.value = it } }
                launch { opened.callsAllowed.collect { mutableCallsAllowed.value = it } }
                launch { opened.openLinks.collect { mutableOpenLinks.value = it } }
                launch { opened.callsAccess.collect { mutableCallsAccess.value = it } }
                launch { opened.callsReject.collect { mutableCallsReject.value = it } }
                opened.state.collect {
                    mutableState.value = it
                    if (it is ConnectionState.Connected) FileReceiving.visibilityChanged(application)
                }
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
