package io.github.prejas3.windconnect

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Bundle
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.core.WindCore
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.delay
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.FixMethodOrder
import org.junit.Test
import org.junit.runners.MethodSorters
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * WN-103: a browser started right after discovery starts sees the phone within 5 s, run after
 * run. The initial callbacks Android sends for the network it already has (`onAvailable`,
 * `onLinkPropertiesChanged`) must not restart mDNS (on Windows a restart ~0.8 s after start
 * hid the device for 5 s in half the runs, WN-89). A real network change still restarts it.
 * The browser is the system's own (NsdManager) on the same device; `-e runs N` (default 10).
 * Needs connected Wi-Fi: NsdManager does no mDNS on cellular. The Wi-Fi toggle runs last.
 */
@FixMethodOrder(MethodSorters.NAME_ASCENDING)
class MdnsStartupTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val nsd = context.getSystemService(Context.NSD_SERVICE) as NsdManager

    private fun shell(command: String): String {
        val descriptor = instrumentation.uiAutomation.executeShellCommand(command)
        return ParcelFileDescriptor.AutoCloseInputStream(descriptor).use { String(it.readBytes()) }
    }

    private fun wifiConnected() = shell("cmd wifi status").lines().any { it.trim().startsWith("Wifi is connected") }

    /** One NsdManager browse for `_windconnect._udp`; counts down when [instance] is found or lost. */
    private inner class Browser(private val instance: String) : AutoCloseable {
        val found = CountDownLatch(1)
        val lost = CountDownLatch(1)
        private val started = CountDownLatch(1)
        private val listener = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(serviceType: String) { started.countDown() }
            override fun onDiscoveryStopped(serviceType: String) {}
            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) { started.countDown() }
            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {}
            override fun onServiceFound(info: NsdServiceInfo) { if (info.serviceName == instance) found.countDown() }
            override fun onServiceLost(info: NsdServiceInfo) { if (info.serviceName == instance) lost.countDown() }
        }
        init {
            nsd.discoverServices("_windconnect._udp", NsdManager.PROTOCOL_DNS_SD, listener)
            started.await(2, TimeUnit.SECONDS)
        }
        override fun close() { try { nsd.stopServiceDiscovery(listener) } catch (_: IllegalArgumentException) {} }
    }

    @Test fun a_browserSeesPhoneRightAfterStart() = runBlocking {
        assertTrue("precondition: Wi-Fi connected (the emulator's may not rejoin after a toggle)", wifiConnected())
        val runs = InstrumentationRegistry.getArguments().getString("runs")?.toInt() ?: 10
        val results = Bundle()
        val found = mutableListOf<Long>()
        val restarts = mutableListOf<Long>()
        repeat(runs) { run ->
            val core = WindCore.open(context, "Android")
            val instance = "wc-" + core.identity.fingerprint.take(40)
            val start = SystemClock.elapsedRealtime()
            var browser: Browser? = null
            try {
                browser = Browser(instance)
                val seen = browser.found.await(5, TimeUnit.SECONDS)
                found += if (seen) SystemClock.elapsedRealtime() - start else -1
                // Initial network callbacks arrive within ~1 s; look a little longer.
                delay((3_000 - (SystemClock.elapsedRealtime() - start)).coerceAtLeast(0))
                restarts += core.network.value.generation.toLong()
            } finally {
                core.shutdown()
            }
            // The goodbye clears the system browser's cache, so the next run sees only a live answer.
            browser?.use { it.lost.await(3, TimeUnit.SECONDS) }
            delay(500)
            results.putString("run$run", "foundMs=${found.last()} generationAfter3s=${restarts.last()}")
        }
        results.putString("foundMs", found.joinToString(","))
        results.putString("generationAfter3s", restarts.joinToString(","))
        instrumentation.sendStatus(0, results)
        assertTrue("browser saw the phone within 5 s in every run: $found", found.all { it in 0..5_000 })
        assertTrue("no network generation from the initial callbacks: $restarts", restarts.all { it == 0L })
    }

    /**
     * WN-111: mobile data going to the background is no network change. An app that asked for
     * mobile data lets go; Android takes FOREGROUND off the still-connected network and the
     * callback reports it lost, then available when an app asks again. Interface and addresses
     * stay, so discovery must not restart. Needs a mobile network (the emulator has one).
     */
    @Test fun a2_mobileDataToTheBackgroundIsNoChange() = runBlocking {
        val connectivity = context.getSystemService(ConnectivityManager::class.java)
        val results = Bundle()
        val core = WindCore.open(context, "Android")
        try {
            delay(3_000)
            val before = core.network.value.generation
            var backgrounded = 0
            repeat(3) {
                val cellular = CompletableDeferred<Network>()
                val request = object : ConnectivityManager.NetworkCallback() {
                    override fun onAvailable(network: Network) { cellular.complete(network) }
                }
                // The app doesn't declare CHANGE_NETWORK_STATE; the test borrows the shell's.
                instrumentation.uiAutomation.adoptShellPermissionIdentity(android.Manifest.permission.CHANGE_NETWORK_STATE)
                try {
                    connectivity.requestNetwork(NetworkRequest.Builder()
                        .addTransportType(NetworkCapabilities.TRANSPORT_CELLULAR)
                        .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET).build(), request)
                } finally { instrumentation.uiAutomation.dropShellPermissionIdentity() }
                val network = try { withTimeoutOrNull(5_000) { cellular.await() } } finally { delay(1_000) }
                connectivity.unregisterNetworkCallback(request)
                assumeTrue("a mobile network to request", network != null)
                // Background unless another app holds mobile data right now.
                val start = SystemClock.elapsedRealtime()
                while (SystemClock.elapsedRealtime() - start < 2_000) {
                    val caps = connectivity.getNetworkCapabilities(network!!)
                    if (caps != null && !caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_FOREGROUND)) { backgrounded++; break }
                    delay(50)
                }
                delay(1_500)
            }
            val after = core.network.value.generation
            results.putString("backgrounded", "$backgrounded/3")
            results.putString("generationBefore", "$before")
            results.putString("generationAfter", "$after")
            instrumentation.sendStatus(0, results)
            assumeTrue("mobile data went to the background at least once (another app may hold it): $results", backgrounded > 0)
            assertTrue("mobile data to the background and back restarts nothing: $results", after == before)
        } finally {
            core.shutdown()
        }
    }

    /** A real network change (Wi-Fi off and on) still raises the generation and the phone is seen again. */
    @Test fun b_networkChangeStillRestartsDiscovery() = runBlocking {
        assertTrue("precondition: Wi-Fi connected", wifiConnected())
        val results = Bundle()
        val core = WindCore.open(context, "Android")
        val instance = "wc-" + core.identity.fingerprint.take(40)
        try {
            delay(3_000)
            val before = core.network.value.generation
            try {
                shell("svc wifi disable")
                val off = SystemClock.elapsedRealtime()
                while (core.network.value.generation == before) {
                    assertTrue("Wi-Fi off restarts discovery in 5 s", SystemClock.elapsedRealtime() - off < 5_000)
                    delay(100)
                }
                results.putString("restartAfterWifiOffMs", (SystemClock.elapsedRealtime() - off).toString())
            } finally {
                shell("svc wifi enable")
            }
            val onGeneration = core.network.value.generation
            val on = SystemClock.elapsedRealtime()
            while (core.network.value.generation == onGeneration) {
                assertTrue("Wi-Fi on restarts discovery in 15 s", SystemClock.elapsedRealtime() - on < 15_000)
                delay(100)
            }
            var settledMs = SystemClock.elapsedRealtime() - on
            results.putString("restartAfterWifiOnMs", settledMs.toString())
            // Every later restart, as ms after Wi-Fi on: the network settles in steps (addresses).
            val steps = mutableListOf<String>()
            var last = core.network.value.generation
            val browser = Browser(instance)
            var seenMs = -1L
            try {
                // Wi-Fi on settles in steps (restarts ~0.6 s apart: new addresses). Since WN-104 (one
                // goodbye per burst, the announcement 1 s after it) the phone is seen ~1.2 s after the
                // last step, one probing; WN-107 asserts it, measured from that last step.
                val until = SystemClock.elapsedRealtime() + 20_000
                while (SystemClock.elapsedRealtime() < until) {
                    val now = core.network.value.generation
                    if (now != last) { settledMs = SystemClock.elapsedRealtime() - on; steps += "$settledMs"; last = now }
                    if (seenMs < 0 && browser.found.count == 0L) seenMs = SystemClock.elapsedRealtime() - on
                    delay(50)
                }
            } finally { browser.close() }
            results.putString("laterRestartsMs", steps.joinToString(","))
            results.putString("seenAfterWifiOnMs", seenMs.toString())
            results.putString("seenAfterSettledMs", (seenMs - settledMs).toString())
            instrumentation.sendStatus(0, results)
            assertTrue("seen again after Wi-Fi on: $results", seenMs >= 0)
            assertTrue("seen within 5 s after Wi-Fi settled (last restart at $settledMs ms, seen at $seenMs ms)", seenMs - settledMs <= 5_000)
        } finally {
            core.shutdown()
            // Leave the device as found: Wi-Fi back and joined, or say so.
            shell("svc wifi enable")
            val start = SystemClock.elapsedRealtime()
            while (!wifiConnected() && SystemClock.elapsedRealtime() - start < 30_000) delay(500)
        }
        assertTrue("Wi-Fi rejoined after the test", wifiConnected())
        instrumentation.sendStatus(0, results)
    }
}
