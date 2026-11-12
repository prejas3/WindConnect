package io.github.prejas3.windconnect

import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.core.ConnectionState
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test

/**
 * CCS/1 PURPOSE:WN-42 "a notification leaves the phone < 50 ms after it appears" (PROTOCOL.md §9.7) against notify_peer (a PC that
 * takes notify/1) at pcAddress/pcKey: the real WindCore connects, then [NotifyProof] runs. notify_peer's output is checked by
 * apps/android/scripts/check-notify-android.cjs. The real PC app's side is WN-43's check-notify-toasts-android.cjs.
 */
class NotifyProofTest {
    private fun hex(text: String) = ByteArray(text.length / 2) { text.substring(it * 2, it * 2 + 2).toInt(16).toByte() }

    @Test fun notificationsLeaveThePhoneFast() {
        val args = InstrumentationRegistry.getArguments()
        val key = args.getString("pcKey")
        val address = args.getString("pcAddress")
        assumeTrue("requires pcKey and pcAddress (notify_peer)", key != null && address != null)
        val app = InstrumentationRegistry.getInstrumentation().targetContext.applicationContext as WindApplication
        val core = runBlocking { app.connections.acquire() }
        try {
            runBlocking { core.connect(address!!, hex(key!!)) }
            assertTrue(core.state.value is ConnectionState.Connected)
            NotifyProof.run(core, args.getString("tag") ?: "proof", args.getString("measure"))
        } finally {
            runBlocking { core.disconnect(); app.connections.release(core) }
        }
    }
}
