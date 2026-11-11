package io.github.prejas3.windconnect

import android.os.Bundle
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.core.ConnectionState
import io.github.prejas3.windconnect.core.WindCore
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test

/**
 * WN-83 / WC-30: the system's own record of multicast locks (`dumpsys wifi`) shows the
 * app's lock only while discovery is needed. Needs a PC peer (README, `android_peer`);
 * `-e toggleWifi true` adds a real network change.
 */
class MulticastLockTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    private fun shell(command: String): String {
        val descriptor = instrumentation.uiAutomation.executeShellCommand(command)
        return ParcelFileDescriptor.AutoCloseInputStream(descriptor).use { String(it.readBytes()) }
    }

    /** The app's lock in Wi-Fi service's held list (indented entries under its header); the
     * dump also carries log lines naming the tag, which say nothing about now. */
    private fun held(): Boolean = shell("dumpsys wifi").lineSequence()
        .dropWhile { it.trim() != "Multicast Locks held:" }.drop(1)
        .takeWhile { it.startsWith(" ") || it.startsWith("\t") }
        .any { "WindConnect discovery" in it }

    private suspend fun until(what: String, millis: Long, wanted: Boolean): Long {
        val start = SystemClock.elapsedRealtime()
        while (held() != wanted) {
            assertTrue("$what: lock held=${!wanted} after ${millis} ms",
                SystemClock.elapsedRealtime() - start < millis)
            delay(200)
        }
        return SystemClock.elapsedRealtime() - start
    }

    @Test fun lockFollowsDiscoveryNeedNotTheAppsLife() = runBlocking {
        val args = InstrumentationRegistry.getArguments()
        val key = args.getString("pcKey")
        val address = args.getString("pcAddress")
        assumeTrue("requires pcKey and pcAddress instrumentation arguments", key != null && address != null)
        val toggleWifi = args.getString("toggleWifi") == "true"
        val results = Bundle()
        val core = WindCore.open(instrumentation.targetContext, "Android")
        try {
            until("start, no session", 2_000, wanted = true)

            val peerKey = key!!.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
            core.connect(address!!, peerKey)
            withTimeout(20_000) { core.state.first { it is ConnectionState.Connected } }
            results.putString("releasedAfterConnectMs", until("active session", 2_000, wanted = false).toString())

            core.refreshDiscovery()
            delay(1_000)
            assertFalse("a network event with an active session takes no lock", held())

            core.discoveryScreen(true)
            until("SAS / device list screen open", 2_000, wanted = true)
            core.discoveryScreen(false)
            until("screen closed, session active", 2_000, wanted = false)

            core.disconnect()
            results.putString("heldAfterSessionLostMs", until("session lost", 2_000, wanted = true).toString())
            // The 30 s window (MulticastPolicy.DEFAULT_WINDOW_MILLIS) ends by itself.
            results.putString("releasedAfterWindowMs", until("window over", 35_000, wanted = false).toString())

            if (toggleWifi) {
                try {
                    shell("svc wifi disable")
                    results.putString("heldAfterNetworkChangeMs",
                        until("network change, no session", 5_000, wanted = true).toString())
                } finally { shell("svc wifi enable") }
            }
        } finally {
            core.shutdown()
        }
        until("shutdown", 2_000, wanted = false)
        instrumentation.sendStatus(0, results)
    }

    /**
     * WN-90: during a session with one PC, another PC is in [WindCore.network] within 5 s of a
     * network change, and the lock stays released. `otherPc` is the second PC's fingerprint
     * (64 hex); the runner (apps/android/scripts/check-multi-pc.cjs) sends its discovery packet
     * every 2 s through the emulator's UDP redirect, like a PC's query period.
     * LIMIT:on the emulator the packet comes in unsolicited through NAT; the phone's query and
     * the PC's unicast answer are covered on the host
     * (`query_is_answered_by_unicast_so_the_asker_needs_no_broadcast`).
     */
    @Test fun otherPcFoundDuringSessionWithoutLock() = runBlocking {
        val args = InstrumentationRegistry.getArguments()
        val key = args.getString("pcKey")
        val address = args.getString("pcAddress")
        val other = args.getString("otherPc")?.lowercase()
        assumeTrue("requires pcKey, pcAddress and otherPc instrumentation arguments",
            key != null && address != null && other != null)
        val results = Bundle()
        val core = WindCore.open(instrumentation.targetContext, "Android")
        try {
            val peerKey = key!!.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
            core.connect(address!!, peerKey)
            withTimeout(20_000) { core.state.first { it is ConnectionState.Connected } }
            until("active session", 2_000, wanted = false)

            // Checks the lock every 200 ms while it waits: a session never takes it.
            suspend fun otherFound(what: String, generation: ULong): Long {
                val start = SystemClock.elapsedRealtime()
                while (core.network.value.generation < generation ||
                    core.network.value.peers.none { it.fingerprint == other }) {
                    assertFalse("$what: the lock is taken during a session", held())
                    assertTrue("$what: other PC not found after 5 s; network=${core.network.value}", SystemClock.elapsedRealtime() - start < 5_000)
                    delay(200)
                }
                return SystemClock.elapsedRealtime() - start
            }
            results.putString("otherFoundInSessionMs", otherFound("session", 0uL).toString())

            for (step in listOf("svc wifi disable", "svc wifi enable")) {
                val before = core.network.value.generation
                shell(step)
                // The network change clears the cache; the PC must come back in it.
                withTimeout(10_000) { core.network.first { it.generation > before } }
                results.putString("otherFoundAfter${step.substringAfterLast(' ')}Ms",
                    otherFound(step, before + 1uL).toString())
            }
            withTimeout(10_000) { core.state.first { it is ConnectionState.Connected } }
            assertFalse("session after the network changes holds no lock", held())
            results.putString("sessionKept", "true")
        } finally {
            shell("svc wifi enable")
            core.shutdown()
        }
        instrumentation.sendStatus(0, results)
    }
}
