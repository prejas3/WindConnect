package io.github.prejas3.windconnect

import android.os.Bundle
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.core.WindCore
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout

/**
 * CCS/1 PURPOSE:WN-55 the phone side of check-sms-android.cjs: "SMS na komputerze" on (READ_SMS/READ_CONTACTS granted by
 * the script), the session held while the PC reads threads; commands from cache/wn55-cmd: "off", "on", "send-on", "send-off" (WN-56), "calls-on", "calls-off" (WN-57), "links-on", "links-off", "pc <id> [url]", "pc-list" (WN-100), "done". No content.
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
                    val line = command.readText().trim()
                    command.delete()
                    // WN-100: "pc <id> [url]" runs a PC action, "pc-list" lists them; the answer goes to cache/wn100-ack.
                    if (line.startsWith("pc ") || line == "pc-list") {
                        val parts = line.split(' ')
                        val answer = if (line == "pc-list") core.pcActions().joinToString(",") { it.id }
                            else core.runPcAction(parts[1], parts.getOrNull(2))
                        java.io.File(context.cacheDir, "wn100-ack.tmp").apply { writeText("$line=$answer") }
                            .renameTo(java.io.File(context.cacheDir, "wn100-ack"))
                        continue
                    }
                    val word = line
                    when (word) {
                        "off" -> core.setSmsAllowed(false)
                        "on" -> core.setSmsAllowed(true)
                        "send-on" -> { core.refreshSms(); check(core.smsSendAccess.value) { "SEND_SMS not granted" }; core.setSmsSendAllowed(true) }
                        "send-off" -> core.setSmsSendAllowed(false)
                        "calls-on" -> { core.refreshCalls(); check(core.callsAccess.value) { "call permissions not granted" }; core.setCallsAllowed(true) }
                        "calls-off" -> core.setCallsAllowed(false)
                        "links-on" -> core.setOpenLinks(true)
                        "links-off" -> core.setOpenLinks(false)
                        "done" -> break
                    }
                }
            }
        } finally {
            command.delete()
            core.setSmsSendAllowed(false)
            core.setCallsAllowed(false)
            core.setOpenLinks(true)
            core.setSmsAllowed(false)
        }
    }
}
