package io.github.prejas3.windconnect.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** CCS/1 SEC:Exercise production mute ownership and refusal without changing the host's audio. */
class RingMuteTest {
    private class Audio : RingMuteAccess {
        override var allowed = true
        override var muted = false
        override var active = false
        var deniedByPolicy = false
        var adjustments = 0
        override fun adjust(mute: Boolean) {
            adjustments++
            if (deniedByPolicy) throw SecurityException("Notification policy denied")
            muted = mute
        }
    }

    @Test fun permissionGrantedMutesAndRestoresOnlyOurMute() {
        val audio = Audio()
        val ring = RingMute(audio)
        assertTrue(ring.canMute())
        assertTrue(ring.mute())
        assertTrue(audio.muted)
        assertTrue(ring.active)
        assertFalse(ring.mute())
        assertEquals(1, audio.adjustments)
        ring.restore()
        assertFalse(audio.muted)
        assertFalse(ring.active)
        audio.muted = true
        assertFalse(ring.mute())
        ring.restore()
        assertTrue(audio.muted)
        assertEquals(2, audio.adjustments)
    }

    @Test fun missingPermissionRefusesWithoutTouchingAudioOrOwnership() {
        val audio = Audio().apply { allowed = false }
        val ring = RingMute(audio)
        assertFalse(ring.canMute())
        assertFalse(ring.mute())
        assertFalse(ring.active)
        assertFalse(audio.muted)
        assertEquals(0, audio.adjustments)
    }

    @Test fun policyDenialIsCaughtAndDoesNotClaimAMute() {
        val audio = Audio().apply { deniedByPolicy = true }
        assertFalse(RingMute(audio).mute())
        assertFalse(audio.active)
        assertFalse(audio.muted)
    }

    @Test fun deniedRestoreKeepsOwnershipUntilRecoveryCanRetry() {
        val audio = Audio()
        assertTrue(RingMute(audio).mute())
        audio.deniedByPolicy = true
        RingMute(audio).recover(ringingNow = false)
        assertTrue(audio.active)
        audio.deniedByPolicy = false
        audio.allowed = false
        RingMute(audio).recover(ringingNow = false)
        assertTrue(audio.active)
        audio.allowed = true
        RingMute(audio).recover(ringingNow = true)
        assertTrue(audio.muted)
        RingMute(audio).recover(ringingNow = false)
        assertFalse(audio.muted)
        assertFalse(audio.active)
    }
}
