package io.github.prejas3.windconnect

import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.bindings.CoreException
import io.github.prejas3.windconnect.bindings.IdentityStorage
import io.github.prejas3.windconnect.bindings.openClient
import io.github.prejas3.windconnect.core.ConnectionState
import io.github.prejas3.windconnect.core.WindCore
import java.net.DatagramSocket
import java.net.InetAddress
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.flow.first
import android.os.Bundle
import android.os.ParcelFileDescriptor
import kotlinx.coroutines.delay
import org.junit.Assume.assumeTrue
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class CoreApiTest {
    @Test fun discoveryCannotAuthorizeAnUnpairedKey() = runBlocking {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val directory = java.io.File(context.cacheDir, "unpaired-${java.util.UUID.randomUUID()}").apply { mkdirs() }
        val scoped = object : android.content.ContextWrapper(context) { override fun getNoBackupFilesDir(): java.io.File = directory }
        val core = WindCore.open(scoped, "Android")
        try {
            try {
                core.connectPaired(io.github.prejas3.windconnect.bindings.PeerInfo(ByteArray(32), "Untrusted", "untrusted"))
                fail("discovery must not authorize an unpaired key")
            } catch (error: IllegalStateException) { assertTrue(error.message.orEmpty().contains("not paired")) }
            assertEquals(ConnectionState.Ready, core.state.value)
        } finally {
            core.shutdown()
            assertTrue(directory.canonicalPath.startsWith(context.cacheDir.canonicalPath + java.io.File.separator))
            directory.deleteRecursively()
        }
    }

    @Test fun damagedPairingRecordsDoNotBlockWindCoreOpen() = runBlocking {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val directory = java.io.File(context.cacheDir, "paired-recovery-${java.util.UUID.randomUUID()}").apply { mkdirs() }
        val scoped = object : android.content.ContextWrapper(context) {
            override fun getNoBackupFilesDir(): java.io.File = directory
        }
        val key = "58" + "66".repeat(31)
        java.io.File(directory, "paired.txt").writeText("\n$key 1 Saved-PC\nbroken record\n")
        try {
            val core = WindCore.open(scoped, "Android")
            try {
                assertEquals(1uL, core.skippedPairingRecords)
                assertEquals(ConnectionState.Ready, core.state.value)
            } finally { core.shutdown() }
        } finally {
            assertTrue(directory.canonicalPath.startsWith(context.cacheDir.canonicalPath + java.io.File.separator))
            directory.deleteRecursively()
        }
    }

    /** CCS/1 RULE:opt-in only; changes Wi-Fi on the explicitly selected test device. */
    @Test fun wifiReturnWakesNativeDiscovery() = runBlocking {
        assumeTrue(InstrumentationRegistry.getArguments().getString("toggleWifi") == "true")
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        fun wifi(enabled: Boolean) {
            val descriptor = instrumentation.uiAutomation.executeShellCommand(
                "svc wifi ${if (enabled) "enable" else "disable"}")
            ParcelFileDescriptor.AutoCloseInputStream(descriptor).use { it.readBytes() }
        }
        val core = WindCore.open(instrumentation.targetContext, "Android")
        try {
            delay(1500) // the observer's replay is no change (WN-103); wait for it to settle
            val before = core.network.value.generation
            wifi(false)
            withTimeout(5000) { core.network.first { it.generation > before } }
            val offline = core.network.value.generation
            val started = System.nanoTime()
            wifi(true)
            val online = withTimeout(5000) { core.network.first { it.generation > offline } }
            instrumentation.sendStatus(0, Bundle().apply {
                putString("wifiReturnGeneration", online.generation.toString())
                putString("wifiReturnWakeMs", ((System.nanoTime() - started) / 1_000_000).toString())
            })
        } finally {
            wifi(true)
            core.shutdown()
        }
    }

    /** WN-103: the callback's replay of the networks it already has is no change, so discovery
     * keeps its first generation (a restart then hid the phone from browsers); real changes are
     * covered by [wifiReturnWakesNativeDiscovery] and MdnsStartupTest. */
    @Test fun platformNetworkCallbackReplayDoesNotRestartDiscovery() = runBlocking {
        val core = WindCore.open(InstrumentationRegistry.getInstrumentation().targetContext, "Android")
        try {
            delay(3000)
            val state = core.network.value
            assertEquals(0uL, state.generation)
            InstrumentationRegistry.getInstrumentation().sendStatus(0, Bundle().apply {
                putString("networkGeneration", state.generation.toString())
            })
        } finally { core.shutdown() }
    }

    @Test fun callbackFailureDoesNotCreateAnotherIdentity() = runBlocking {
        val storage = object : IdentityStorage {
            override fun load(): ByteArray? = throw CoreException.Storage("Keystore unavailable")
            override fun storeNew(pem: ByteArray): Boolean = error("must not replace the identity")
        }
        try {
            openClient(storage, "Android", "127.0.0.1:0").also { it.shutdown(); it.destroy() }
            fail("storage error must cross the FFI boundary")
        } catch (error: CoreException.Storage) {
            assertTrue(error.reason.contains("Keystore unavailable"))
        }
    }

    @Test fun cancellingConnectReleasesTheFutureAndShutdownStopsTheClient() = runBlocking {
        val core = WindCore.open(InstrumentationRegistry.getInstrumentation().targetContext, "Android")
        try {
            // CCS/1 DEC:silent UDP socket keeps QUIC pending without an external network dependency.
            DatagramSocket(0, InetAddress.getByName("127.0.0.1")).use { silent ->
                try {
                    withTimeout(100) {
                        core.connect("127.0.0.1:${silent.localPort}", core.identity.publicKey)
                    }
                    fail("connection must be cancelled")
                } catch (_: TimeoutCancellationException) {
                    assertTrue(core.state.value is ConnectionState.Disconnected)
                }
            }
        } finally {
            core.shutdown()
        }
        assertEquals(ConnectionState.Stopped, core.state.value)
    }
}
