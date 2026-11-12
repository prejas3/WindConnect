package io.github.prejas3.windconnect

import android.os.Bundle
import android.os.SystemClock
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.core.NotifyListenerService
import io.github.prejas3.windconnect.core.NotifySend
import io.github.prejas3.windconnect.core.WindCore
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import java.util.Collections

/**
 * CCS/1 PURPOSE:WN-42/WN-43 the phone side of the notification checks, on a connected [core]. The setting goes on, the shell posts
 * (another app, as a real one would) 20 plain notifications, a chat, a media one (skipped), an unchanged re-post (dropped), then
 * snoozes the second (Removed). Reports phone latencies ("notifyProof") and each sent one's listener and post wall times
 * ("notifyTimes", for the PC side). Asserts p95 < 50 ms on the phone for `measure` ("listener" or "post"). Tags start with [tag]:
 * an earlier run's notifications (one snoozed) stay on the phone.
 */
object NotifyProof {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    fun shell(command: String): String =
        instrumentation.uiAutomation.executeShellCommand(command).let { fd ->
            android.os.ParcelFileDescriptor.AutoCloseInputStream(fd).use { it.readBytes().decodeToString() }
        }

    fun run(core: WindCore, tag: String, measure: String?) = runBlocking {
        assertTrue("the script grants NotifyListenerService's access", NotifyListenerService.granted(instrumentation.targetContext))
        val sends = Collections.synchronizedList(mutableListOf<NotifySend>())
        val collector = launch(Dispatchers.IO) { core.notifySends.collect { sends.add(it) } }
        try {
            core.setNotifyAllowed(true)
            withTimeout(10_000) { while (!core.notifyAccess.value) { core.refreshNotify(); delay(100) } }
            // Let the system rebind the listener after the setting went on.
            delay(1_500)
            fun waitSends(count: Int, label: String) {
                val deadline = SystemClock.elapsedRealtime() + 10_000
                while (sends.size < count) { assertTrue("$label: ${sends.size}/$count", SystemClock.elapsedRealtime() < deadline); Thread.sleep(10) }
            }
            for (i in 1..20) {
                // Letters, not numbers: with an assistant Android 15 may redact number-like texts as sensitive (OTP).
                val c = 'a' + (i - 1)
                shell("cmd notification post -t Ola_$c $tag$i Wiadomosc_$c")
                waitSends(i, "plain $i")
                Thread.sleep(150)
            }
            shell("cmd notification post -S messaging --conversation Rodzina --message Ola:hej --message Piotr:klucze ${tag}chat x")
            waitSends(21, "messaging")
            shell("cmd notification post -S media -t Piosenka ${tag}media y")
            Thread.sleep(1_000)
            assertEquals("a player is skipped", 21, sends.size)
            shell("cmd notification post -t Ola_a ${tag}1 Wiadomosc_a")
            waitSends(22, "unchanged re-post")
            assertEquals("an unchanged re-post is dropped", "dropped", sends[21].outcome)
            val list = shell("cmd notification list")
            val proof2 = list.lines().map { it.trim() }.first { it.contains("|com.android.shell|") && it.contains("|${tag}2|") }
            shell("cmd notification snooze --for 60000 $proof2")
            Thread.sleep(1_000)

            val sent = sends.take(21)
            assertTrue("all sent: ${sent.map { it.outcome }}", sent.all { it.outcome == "sent" })
            val postToDone = sent.map { it.doneMs - it.postTimeMs }.sorted()
            val listenerToDone = sent.map { it.listenerToDoneMs }.sorted()
            fun p(list: List<Long>, q: Double) = list[minOf(list.size - 1, (q * list.size).toInt())]
            val report = "n=${sent.size} postToSent p50=${p(postToDone, 0.5)} p95=${p(postToDone, 0.95)} max=${postToDone.last()} " +
                "listenerToSent p50=${p(listenerToDone, 0.5)} p95=${p(listenerToDone, 0.95)} max=${listenerToDone.last()} " +
                "all=${sent.map { it.doneMs - it.postTimeMs }}"
            instrumentation.sendStatus(0, Bundle().apply {
                putString("notifyProof", report)
                // In sending order: the listener's wall time (callback) and Android's postTime, ms.
                putString("notifyTimes", sent.joinToString(",") { "${it.doneMs - it.listenerToDoneMs}:${it.postTimeMs}" })
            })
            // Android may hold a new notification ~200 ms for its notification assistant before any listener, SystemUI's shade
            // included, sees it: "listener" measures from the callback; "post" (the script removed the assistant) from postTime.
            val measured = if (measure == "post") postToDone else listenerToDone
            assertTrue(report, p(measured, 0.95) < 50)
        } finally {
            collector.cancel()
            core.setNotifyAllowed(false)
        }
    }
}
