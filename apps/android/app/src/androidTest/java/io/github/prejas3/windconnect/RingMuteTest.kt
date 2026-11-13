package io.github.prejas3.windconnect

import android.media.AudioManager
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.core.RingMute
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/** WN-57 review: "Wycisz" from the PC never leaves the phone's ring stream muted, and a mute the person set stays theirs. */
@RunWith(AndroidJUnit4::class)
class RingMuteTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val audio = context.getSystemService(AudioManager::class.java)
    private fun muted() = audio.isStreamMute(AudioManager.STREAM_RING)

    @Before fun unmuted() {
        audio.adjustStreamVolume(AudioManager.STREAM_RING, AudioManager.ADJUST_UNMUTE, 0)
        RingMute(context).restore()
        if (audio.getStreamVolume(AudioManager.STREAM_RING) == 0) audio.setStreamVolume(AudioManager.STREAM_RING, 3, 0)
    }

    @After fun restore() = unmuted()

    @Test fun ourMuteIsUndoneAtTheCallsEnd() {
        val mute = RingMute(context)
        assertTrue(mute.mute())
        assertTrue(muted())
        mute.restore()
        assertFalse(muted())
        assertFalse(mute.active)
    }

    @Test fun aRestartUndoesItWhenTheCallIsOver() {
        assertTrue(RingMute(context).mute())
        // A new process: the record survives; still ringing, it stays; idle, it goes.
        val next = RingMute(context)
        assertTrue(next.active)
        next.recover(ringingNow = true)
        assertTrue(muted())
        next.recover(ringingNow = false)
        assertFalse(muted())
        assertFalse(RingMute(context).active)
    }

    @Test fun thePersonsOwnMuteIsLeftAlone() {
        audio.adjustStreamVolume(AudioManager.STREAM_RING, AudioManager.ADJUST_MUTE, 0)
        val mute = RingMute(context)
        assertFalse("already muted: not ours", mute.mute())
        assertFalse(mute.active)
        mute.recover(ringingNow = false)
        mute.restore()
        assertTrue("the person's mute stays", muted())
    }
}
