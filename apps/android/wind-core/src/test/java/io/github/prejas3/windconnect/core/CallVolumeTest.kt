package io.github.prejas3.windconnect.core

import android.media.AudioManager
import android.telephony.TelephonyManager
import org.junit.Assert.assertEquals
import org.junit.Test

/** WN-48: the call-state mapping and STREAM_MUSIC scaling. */
class CallVolumeTest {
    @Test fun audioModesAndTelephonyStates() {
        assertEquals(CallState.Ringing, CallStates.fromMode(AudioManager.MODE_RINGTONE))
        assertEquals(CallState.Offhook, CallStates.fromMode(AudioManager.MODE_IN_CALL))
        assertEquals("messengers too", CallState.Offhook, CallStates.fromMode(AudioManager.MODE_IN_COMMUNICATION))
        assertEquals(CallState.Idle, CallStates.fromMode(AudioManager.MODE_NORMAL))
        assertEquals(CallState.Ringing, CallStates.fromTelephony(TelephonyManager.CALL_STATE_RINGING))
        assertEquals(CallState.Offhook, CallStates.fromTelephony(TelephonyManager.CALL_STATE_OFFHOOK))
        assertEquals(CallState.Idle, CallStates.fromTelephony(TelephonyManager.CALL_STATE_IDLE))
        assertEquals(listOf(0, 1, 2), CallState.entries.map { it.code })
    }

    @Test fun aCallNotificationCountsWhenTheModeDoesNot() {
        // Vibrate/silent: the mode may stay NORMAL while the dialer shows its incoming-call notification.
        assertEquals(CallState.Ringing, CallStates.combine(CallState.Idle, callNotification = true))
        assertEquals(CallState.Offhook, CallStates.combine(CallState.Offhook, callNotification = true))
        assertEquals(CallState.Idle, CallStates.combine(CallState.Idle, callNotification = false))
    }

    @Test fun scalingMatchesTheCore() {
        // The same numbers as wind-core's volume.rs tests.
        assertEquals(0, VolumeScale.toSteps(0, 15)); assertEquals(15, VolumeScale.toSteps(100, 15)); assertEquals(8, VolumeScale.toSteps(50, 15))
        assertEquals(53, VolumeScale.toPercent(8, 15)); assertEquals(100, VolumeScale.toPercent(15, 15)); assertEquals(0, VolumeScale.toPercent(3, 0))
        for (max in listOf(7, 15, 25, 100)) for (steps in 0..max) assertEquals(steps, VolumeScale.toSteps(VolumeScale.toPercent(steps, max), max))
    }

    @Test fun aSetChangesOnlyWhatItNames() {
        // A level alone while muted is not applied (it would unmute).
        assertEquals(VolumeScale.Plan(null, unmute = false, mute = false), VolumeScale.plan(80, null, currentlyMuted = true, max = 15))
        assertEquals(VolumeScale.Plan(12, unmute = false, mute = false), VolumeScale.plan(80, null, currentlyMuted = false, max = 15))
        assertEquals(VolumeScale.Plan(12, unmute = true, mute = false), VolumeScale.plan(80, false, currentlyMuted = true, max = 15))
        assertEquals(VolumeScale.Plan(null, unmute = false, mute = true), VolumeScale.plan(null, true, currentlyMuted = false, max = 15))
        assertEquals(VolumeScale.Plan(null, unmute = true, mute = false), VolumeScale.plan(null, false, currentlyMuted = true, max = 15))
    }
}
