package io.github.prejas3.windconnect

import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class ReceiveAdmissionTest {
    private class Fixture(timeout: Long = 1_000) {
        var enabled = true
        var visible = true
        var owner = false
        var failStart = false
        var starts = 0
        var changes = 0
        val gate = ReceiveAdmission({ enabled }, { visible }, { owner }, {
            starts++
            if (failStart) throw IllegalStateException("FGS start denied")
        }, { changes++ }, timeout)
    }

    @Test fun onlyAnAdmissionStartsAServiceAndWaitsForActualOwnership() = runBlocking {
        val f = Fixture()
        assertEquals(0, f.starts)
        val attempt = async(start = CoroutineStart.UNDISPATCHED) { f.gate.begin(1u) }
        assertEquals(1, f.starts)
        assertFalse(attempt.isCompleted)
        f.owner = true; f.gate.ownerChanged()
        assertTrue(attempt.await())
        assertTrue(f.gate.active)
        f.gate.end(1u)
        assertFalse(f.gate.active)
        assertEquals(1, f.changes)
    }

    @Test fun backgroundRequiresARealServiceOwner() = runBlocking {
        val f = Fixture().apply { visible = false }
        assertFalse(f.gate.begin(1u))
        assertEquals(0, f.starts)
        f.owner = true
        assertTrue(f.gate.begin(2u))
        assertEquals(0, f.starts)
        f.gate.end(2u)
        assertFalse(f.gate.active)
    }

    @Test fun deniedServiceStartReleasesItsToken() = runBlocking {
        val f = Fixture().apply { failStart = true }
        assertFalse(f.gate.begin(1u))
        assertFalse(f.gate.active)
        assertEquals(1, f.changes)
    }

    @Test fun timeoutAndLateAcknowledgementCannotReviveOrReleaseANewerAttempt() = runBlocking {
        val f = Fixture(timeout = 30)
        assertFalse(f.gate.begin(1u))
        assertFalse(f.gate.active)
        val newer = async(start = CoroutineStart.UNDISPATCHED) { f.gate.begin(2u) }
        f.owner = true; f.gate.ownerChanged()
        assertTrue(newer.await())
        f.gate.end(1u)
        assertTrue(f.gate.active)
        f.gate.end(2u)
        assertFalse(f.gate.active)
        assertEquals(2, f.changes)
    }

    @Test fun cancellationDoesNotStopAnOverlappingAdmission() = runBlocking {
        val f = Fixture()
        val first = async(start = CoroutineStart.UNDISPATCHED) { f.gate.begin(1u) }
        val second = async(start = CoroutineStart.UNDISPATCHED) { f.gate.begin(2u) }
        first.cancelAndJoin()
        assertTrue(f.gate.active)
        f.owner = true; f.gate.ownerChanged()
        assertTrue(second.await())
        f.gate.end(1u)
        assertTrue(f.gate.active)
        f.gate.end(2u)
        assertFalse(f.gate.active)
    }

    @Test fun lastActivityStopsDuringStartupAndLateOwnerDoesNotReviveIt() = runBlocking {
        val f = Fixture()
        val attempt = async(start = CoroutineStart.UNDISPATCHED) { f.gate.begin(1u) }
        f.visible = false; f.gate.ownerChanged()
        assertFalse(attempt.await())
        f.owner = true; f.gate.ownerChanged()
        assertFalse(f.gate.active)
        assertEquals(1, f.changes)
    }

    @Test fun oneTransferEndingKeepsTheOtherAcceptedPermit() = runBlocking {
        val f = Fixture().apply { owner = true }
        assertTrue(f.gate.begin(1u)); assertTrue(f.gate.begin(2u))
        f.gate.end(1u)
        assertTrue(f.gate.active)
        f.gate.end(2u)
        assertFalse(f.gate.active)
        assertEquals(2, f.changes)
    }

    @Test fun acknowledgementRechecksSettingActualOwnerAndConnectionIdentity() = runBlocking {
        for (mode in 0..2) {
            val f = Fixture()
            var sameConnection = true
            val attempt = async(start = CoroutineStart.UNDISPATCHED) { f.gate.begin(1u) { sameConnection } }
            f.owner = true; f.gate.ownerChanged()
            when (mode) { 0 -> f.enabled = false; 1 -> f.owner = false; else -> sameConnection = false }
            assertFalse(attempt.await())
            assertFalse(f.gate.active)
        }
    }
}
