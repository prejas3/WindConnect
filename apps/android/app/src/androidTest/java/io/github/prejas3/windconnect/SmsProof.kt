package io.github.prejas3.windconnect

import android.os.Bundle
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.core.WindCore
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout

/**
 * CCS/1 PURPOSE:WN-55 the phone side of check-sms-android.cjs: "SMS na komputerze" on (READ_SMS/READ_CONTACTS granted by
 * the script), the session held while the PC reads threads; commands from cache/wn55-cmd: "off", "on", "done". No content.
 */
object SmsProof {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    fun hold(core: WindCore) = runBlocking {
        val context = instrumentation.targetContext
        val command = java.io.File(context.cacheDir, "wn55-cmd").apply { delete() }
        try {
            core.refreshSms()
            check(core.smsAccess.value) { "READ_SMS not granted" }
            core.setSmsAllowed(true)
            instrumentation.sendStatus(0, Bundle().apply { putString("smsProofReady", "true") })
            withTimeout(400_000) {
                while (true) {
                    if (!command.exists()) { delay(100); continue }
                    val word = command.readText().trim()
                    command.delete()
                    when (word) {
                        "off" -> core.setSmsAllowed(false)
                        "on" -> core.setSmsAllowed(true)
                        "done" -> break
                    }
                }
            }
        } finally {
            command.delete()
            core.setSmsAllowed(false)
        }
    }
}
