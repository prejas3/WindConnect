package io.github.prejas3.windconnect

import android.media.AudioManager
import android.os.Bundle
import android.os.Process
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.core.ConnectionState
import io.github.prejas3.windconnect.core.RingMute
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test

/** CCS/1 PURPOSE:host-controlled real process-death fixture; seed deliberately leaves our mute for the manifest receiver. */
class RingMuteCrashFixture {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val audio = context.getSystemService(AudioManager::class.java)

    @Test fun seedMute() {
        assumeTrue(InstrumentationRegistry.getArguments().getString("ringerCrashFixture") == "true")
        assertEquals(ConnectionState.Stopped, (context.applicationContext as WindApplication).connections.state.value)
        audio.adjustStreamVolume(AudioManager.STREAM_RING, AudioManager.ADJUST_UNMUTE, 0)
        if (audio.getStreamVolume(AudioManager.STREAM_RING) == 0) audio.setStreamVolume(AudioManager.STREAM_RING, 3, 0)
        assertTrue(RingMute(context).mute())
        assertTrue(audio.isStreamMute(AudioManager.STREAM_RING))
        instrumentation.sendStatus(0, Bundle().apply { putString("mutedPid", Process.myPid().toString()) })
    }

    @Test fun verifyRestoredWithoutCore() {
        assumeTrue(InstrumentationRegistry.getArguments().getString("ringerCrashFixture") == "true")
        assertEquals(ConnectionState.Stopped, (context.applicationContext as WindApplication).connections.state.value)
        assertFalse("manifest IDLE receiver cleared the persisted flag", RingMute(context).active)
        assertFalse("ring stream is restored", audio.isStreamMute(AudioManager.STREAM_RING))
    }
}
