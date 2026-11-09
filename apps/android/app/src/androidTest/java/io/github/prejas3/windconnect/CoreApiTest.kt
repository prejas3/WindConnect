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
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class CoreApiTest {
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
