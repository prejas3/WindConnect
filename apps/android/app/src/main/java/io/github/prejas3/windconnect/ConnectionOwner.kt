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
    fun setFindAllowed(value: Boolean) { val opened = core ?: return; scope.launch { opened.setFindAllowed(value) } }

    suspend fun acquire(): WindCore = gate.withLock {
        val opened = core ?: WindCore.open(application, android.os.Build.MODEL.take(64)).also { opened ->
            core = opened
            mutableState.value = opened.state.value
            observation = scope.launch {
                launch { opened.ringing.collect { mutableRinging.value = it } }
                launch { opened.findAllowed.collect { mutableFindAllowed.value = it } }
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
            core = null
            opened.shutdown()
        }
    }
}
