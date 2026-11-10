package io.github.prejas3.windconnect

import android.app.Application
import io.github.prejas3.windconnect.core.WindCore
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** CCS/1 RULE:Activity and service lease one native client; only the final release shuts it down. */
class WindApplication : Application() {
    val connections = ConnectionOwner(this)
}

class ConnectionOwner(private val application: Application) {
    private val gate = Mutex()
    private var core: WindCore? = null
    private var leases = 0

    suspend fun acquire(): WindCore = gate.withLock {
        val opened = core ?: WindCore.open(application, android.os.Build.MODEL.take(64)).also { core = it }
        leases++
        opened
    }

    suspend fun release(opened: WindCore) = gate.withLock {
        check(core === opened && leases > 0)
        leases--
        if (leases == 0) {
            core = null
            opened.shutdown()
        }
    }
}
