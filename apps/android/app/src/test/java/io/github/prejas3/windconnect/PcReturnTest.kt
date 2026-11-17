package io.github.prejas3.windconnect

import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** WN-142: when the background reconnect stops waiting for its backoff. */
class PcReturnTest {
    private val old = setOf("192.168.0.10:50001")
    private val restarted = setOf("192.168.0.10:50077")

    @Test fun aPcOnANewAddressEndsTheWaitAtOnce() = runBlocking {
        val seen = MutableStateFlow(old)
        val wait = async { val start = System.nanoTime(); awaitPcReturn(seen, old, 5_000) to (System.nanoTime() - start) / 1_000_000 }
        delay(100)
        seen.value = restarted
        val (back, ms) = wait.await()
        assertTrue(back)
        assertTrue("woke in $ms ms", ms < 2_000)
    }

    @Test fun aPcThatWasGoneAndCameBackOnTheSameAddressEndsTheWait() = runBlocking {
        val seen = MutableStateFlow(old)
        val wait = async { awaitPcReturn(seen, old, 5_000) }
        delay(100); seen.value = emptySet()
        delay(100); seen.value = old
        assertTrue(wait.await())
    }

    @Test fun aPcNotSeenAtTheAttemptWakesTheLoopWhenItAppears() = runBlocking {
        val seen = MutableStateFlow(emptySet<String>())
        val wait = async { awaitPcReturn(seen, emptySet(), 5_000) }
        delay(100); seen.value = old
        assertTrue(wait.await())
    }

    @Test fun theSameAddressStayingPutWaitsOutTheBackoff() = runBlocking {
        // A PC that answers BUSY or refuses us stays where it is: no retry loop (WC-28).
        val seen = MutableStateFlow(old)
        assertFalse(awaitPcReturn(seen, old, 300))
    }

    @Test fun nothingSeenWaitsOutTheBackoff() = runBlocking {
        assertFalse(awaitPcReturn(MutableStateFlow(emptySet()), emptySet(), 300))
    }
}
