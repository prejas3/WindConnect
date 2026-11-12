package io.github.prejas3.windconnect

import android.os.Bundle
import android.os.SystemClock
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.core.ConnectionState
import io.github.prejas3.windconnect.core.NotifySend
import io.github.prejas3.windconnect.core.NotifyListenerService
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import java.util.Collections

/**
 * CCS/1 PURPOSE:WN-42 "a notification leaves the phone < 50 ms after it appears" (PROTOCOL.md §9.7). With notify_peer (a PC that
 * takes notify/1) at pcAddress/pcKey and NotifyListenerService's access granted by the script, the real WindCore connects, the
 * setting goes on, and the shell posts notifications (another app, as a real one would): 20 plain ones, a messaging one, a media
 * one (skipped), an unchanged re-post (dropped), then one is snoozed (Removed). Latency: Android's postTime, and the listener's
 * callback, to the core having written it to the QUIC stream; p95 < 50 ms for the one "measure" names. With a notification
 * assistant Android holds new notifications ~200 ms (NotificationManagerService DELAY_FOR_ASSISTANT_TIME) before every listener,
 * the phone's own shade included: "appears" is then the callback. notify_peer's output is checked by the script.
 */
class NotifyProofTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    private fun shell(command: String): String =
        instrumentation.uiAutomation.executeShellCommand(command).let { fd ->
            android.os.ParcelFileDescriptor.AutoCloseInputStream(fd).use { it.readBytes().decodeToString() }
        }

    private fun hex(text: String) = ByteArray(text.length / 2) { text.substring(it * 2, it * 2 + 2).toInt(16).toByte() }

    @Test fun notificationsLeaveThePhoneFast() = runBlocking {
        val args = InstrumentationRegistry.getArguments()
        val key = args.getString("pcKey")
        val address = args.getString("pcAddress")
        assumeTrue("requires pcKey and pcAddress (notify_peer)", key != null && address != null)
        // Each run its own tags: an earlier run's notifications (one snoozed) stay on the phone.
        val tag = args.getString("tag") ?: "proof"
        val app = instrumentation.targetContext.applicationContext as WindApplication
        assertTrue("the script grants NotifyListenerService's access", NotifyListenerService.granted(app))
        val core = app.connections.acquire()
        val sends = Collections.synchronizedList(mutableListOf<NotifySend>())
        val collector = launch(Dispatchers.IO) { core.notifySends.collect { sends.add(it) } }
        try {
            core.setNotifyAllowed(true)
            withTimeout(10_000) { while (!core.notifyAccess.value) { core.refreshNotify(); kotlinx.coroutines.delay(100) } }
            core.connect(address!!, hex(key!!))
            assertTrue(core.state.value is ConnectionState.Connected)
            // Let the system rebind the listener after the setting went on.
            kotlinx.coroutines.delay(1_500)
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
            instrumentation.sendStatus(0, Bundle().apply { putString("notifyProof", report) })
            // "measure": listener (default; Android may hold a new notification ~200 ms for its notification assistant before any
            // listener, SystemUI's shade included, sees it) or post (the script removed the assistant: postTime itself).
            val measured = if (args.getString("measure") == "post") postToDone else listenerToDone
            assertTrue(report, p(measured, 0.95) < 50)
        } finally {
            collector.cancel()
            core.setNotifyAllowed(false)
            core.disconnect()
            app.connections.release(core)
        }
    }
}
