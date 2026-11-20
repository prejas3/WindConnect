package io.github.prejas3.windconnect

import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.Bundle
import android.os.ParcelFileDescriptor
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.core.ConnectionState
import io.github.prejas3.windconnect.core.WindCore
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test

/**
 * WN-60: on mobile data only (Wi-Fi off), with the relay turned on, the phone reaches the PC
 * through the relay when the PC's home address does not answer; the setting is off by default
 * and survives a restart. Needs `windctl serve --relay` and the relay (README "Relay outside the home").
 */
class RelayRouteTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    private fun shell(command: String) {
        val descriptor = instrumentation.uiAutomation.executeShellCommand(command)
        ParcelFileDescriptor.AutoCloseInputStream(descriptor).use { it.readBytes() }
    }

    private fun onMobileDataOnly(): Boolean {
        val connectivity = instrumentation.targetContext.getSystemService(ConnectivityManager::class.java)
        val caps = connectivity.getNetworkCapabilities(connectivity.activeNetwork) ?: return false
        return caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) &&
            !caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)
    }

    @Test fun clipboardPeerReachedThroughTheRelayOnMobileData() = runBlocking {
        val args = InstrumentationRegistry.getArguments()
        val key = args.getString("pcKey")
        val relay = args.getString("relayUrl")
        val away = args.getString("awayAddress")
        assumeTrue("requires pcKey, relayUrl and awayAddress", key != null && relay != null && away != null)
        val results = Bundle()
        val context = instrumentation.targetContext
        var core = WindCore.open(context, "Android")
        try {
            core.setRelay(null)
            assertNull("off by default (D3)", core.relayUrl.value)
            shell("svc wifi disable")
            withTimeout(15_000) { while (!onMobileDataOnly()) delay(200) }
            results.putString("network", "cellular only")

            core.setRelay(relay)
            core.shutdown()
            core = WindCore.open(context, "Android")
            assertEquals("the setting survives a restart", relay, core.relayUrl.value)

            val peerKey = key!!.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
            val started = System.nanoTime()
            core.connect(away!!, peerKey)
            val connected = withTimeout(20_000) {
                core.state.first { it is ConnectionState.Connected } as ConnectionState.Connected
            }
            assertEquals("relay", connected.route)
            assertEquals("PC", connected.peerName)
            results.putString("route", connected.route)
            results.putString("connectMs", ((System.nanoTime() - started) / 1_000_000).toString())
            results.putString("pingMicros", connected.roundTripMicros.toString())
            core.disconnect()

            core.setRelay(null)
            core.shutdown()
            core = WindCore.open(context, "Android")
            assertNull("turning it off sticks", core.relayUrl.value)
            assertTrue(core.state.value is ConnectionState.Ready)
        } finally {
            shell("svc wifi enable")
            core.shutdown()
        }
        instrumentation.sendStatus(0, results)
    }
}
