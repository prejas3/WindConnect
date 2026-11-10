package io.github.prejas3.windconnect

import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.core.ConnectionState
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class ConnectionOwnerTest {
    @Test fun closingOneScreenDoesNotCloseAnotherOwnersClient() = runBlocking {
        val app = InstrumentationRegistry.getInstrumentation().targetContext.applicationContext as WindApplication
        val owner = ConnectionOwner(app)
        val screen = owner.acquire()
        val service = owner.acquire()
        assertSame(screen, service)
        val key = screen.identity.publicKey
        owner.release(screen)
        assertNotEquals(ConnectionState.Stopped, service.state.value)
        owner.release(service)
        assertEquals(ConnectionState.Stopped, service.state.value)
        val reopened = owner.acquire()
        try { assertArrayEquals(key, reopened.identity.publicKey) } finally { owner.release(reopened) }
    }
}
