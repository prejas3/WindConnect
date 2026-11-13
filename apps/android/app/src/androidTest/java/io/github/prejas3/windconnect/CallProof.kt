package io.github.prejas3.windconnect

import android.media.AudioManager
import android.os.Bundle
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.core.WindCore
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout

/**
 * CCS/1 PURPOSE:WN-48 the phone side of check-call-volume-android.cjs: holds the session and runs the script's commands from
 * cache/wn48-cmd ("pcvol <0-100>", "pcmute <0|1>", "notify on|off", "volume on|off", "done"), answering in cache/wn48-ack with
 * "<n> pc=<level>,<muted> phone=<level>,<muted>": the PC's volume as this phone got it, and STREAM_MUSIC (0..100). No content.
 */
object CallProof {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    fun hold(core: WindCore) = runBlocking {
        val context = instrumentation.targetContext
        val audio = context.getSystemService(AudioManager::class.java)
        val command = java.io.File(context.cacheDir, "wn48-cmd").apply { delete() }
        val ack = java.io.File(context.cacheDir, "wn48-ack").apply { delete() }
        try {
            core.setVolumeAllowed(true)
            delay(1_000)
            instrumentation.sendStatus(0, Bundle().apply { putString("callProofReady", "true") })
            var n = 0
            withTimeout(400_000) {
                while (true) {
                    if (!command.exists()) { delay(100); continue }
                    val parts = command.readText().trim().split(' ')
                    command.delete()
                    when (parts[0]) {
                        "pcvol" -> core.setPcVolume(level = parts[1].toInt())
                        "pcmute" -> core.setPcVolume(muted = parts[1] == "1")
                        "notify" -> {
                            core.setNotifyAllowed(parts[1] == "on")
                            withTimeout(10_000) { while (parts[1] == "on" && !core.notifyAccess.value) { core.refreshNotify(); delay(100) } }
                        }
                        "volume" -> core.setVolumeAllowed(parts[1] == "on")
                        "done" -> break
                    }
                    delay(600)
                    n++
                    val pc = core.pcVolume.value?.let { "${it.level},${it.muted}" } ?: "none"
                    val max = audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
                    val phone = "${(audio.getStreamVolume(AudioManager.STREAM_MUSIC) * 100 + max / 2) / max},${audio.isStreamMute(AudioManager.STREAM_MUSIC)}"
                    java.io.File(context.cacheDir, "wn48-ack.tmp").apply { writeText("$n pc=$pc phone=$phone") }.renameTo(ack)
                }
            }
        } finally {
            command.delete()
            core.setNotifyAllowed(false)
            core.setVolumeAllowed(true)
        }
    }
}
