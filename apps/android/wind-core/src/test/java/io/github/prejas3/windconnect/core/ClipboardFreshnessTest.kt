package io.github.prejas3.windconnect.core

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ClipboardFreshnessTest {
    @Test fun oldClipCannotOverwritePeerAfterProcessRestartOrReconnect() {
        val freshProcess = ClipboardFreshness()
        assertFalse(freshProcess.allows(90))
        freshProcess.sessionStarted(100)
        assertFalse(freshProcess.allows(90))
        assertFalse(freshProcess.allows(100))
        assertTrue(freshProcess.allows(101))
        freshProcess.sessionStarted(120)
        assertFalse(freshProcess.allows(101))
        assertTrue(freshProcess.allows(121))
    }
    @Test fun unknownTimestampAndClockRollbackAreConservativelySkipped() {
        val policy = ClipboardFreshness()
        policy.sessionStarted(100)
        assertFalse(policy.allows(0))
        assertTrue(policy.allows(120))
        assertFalse(policy.allows(95))
    }
}
