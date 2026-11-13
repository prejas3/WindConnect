package io.github.prejas3.windconnect.core

import android.content.Context
import android.database.ContentObserver
import android.media.AudioManager
import android.os.Handler
import android.os.Looper
import android.provider.Settings

/** WN-48: 0..100 for the protocol, Android's 0..max steps on the device (the same rounding as wind-core's volume.rs). */
internal object VolumeScale {
    fun toPercent(steps: Int, max: Int): Int = if (max <= 0) 0 else ((steps.coerceIn(0, max) * 100 + max / 2) / max)
    fun toSteps(percent: Int, max: Int): Int = (percent.coerceIn(0, 100) * max + 50) / 100

    /**
     * What a PC `Set` does to STREAM_MUSIC (only what it names): the steps to set (null: leave), and whether to unmute or mute
     * first. A level alone while muted is kept for the unmute, never unmuting by itself (WN-48 review, point 4).
     */
    data class Plan(val steps: Int?, val unmute: Boolean, val mute: Boolean)

    fun plan(level: Int?, muted: Boolean?, currentlyMuted: Boolean, max: Int): Plan {
        val steps = level?.let { toSteps(it, max) }
        return when {
            muted == true -> Plan(steps, unmute = false, mute = true)
            muted == false -> Plan(steps, unmute = true, mute = false)
            currentlyMuted -> Plan(null, unmute = false, mute = false)
            else -> Plan(steps, unmute = false, mute = false)
        }
    }
}

/**
 * CCS/1 FLOW:WN-48 STREAM_MUSIC -> [onLevel] (on start and on every change, from anywhere: keys, other apps), via a
 * ContentObserver on Settings.System; no polling. [set] applies the PC's `Set` without the system's volume UI. A fixed-volume
 * device ignores `Set`s and still reports its level.
 */
internal class PhoneVolume(context: Context, private val onLevel: (Int, Boolean) -> Unit) {
    private val app = context.applicationContext
    private val audio = app.getSystemService(AudioManager::class.java)
    private val stream = AudioManager.STREAM_MUSIC
    private val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
        override fun onChange(selfChange: Boolean) = report()
    }
    private var last: Pair<Int, Boolean>? = null

    fun start() {
        app.contentResolver.registerContentObserver(Settings.System.CONTENT_URI, true, observer)
        report()
    }

    fun stop() { app.contentResolver.unregisterContentObserver(observer); last = null }

    private fun current(): Pair<Int, Boolean> =
        VolumeScale.toPercent(audio.getStreamVolume(stream), audio.getStreamMaxVolume(stream)) to audio.isStreamMute(stream)

    @Synchronized private fun report() {
        val now = current()
        if (now == last) return
        last = now
        onLevel(now.first, now.second)
    }

    /** The PC's `Set` (only what it names). */
    fun set(level: Int?, muted: Boolean?) {
        if (audio.isVolumeFixed) return
        val plan = VolumeScale.plan(level, muted, audio.isStreamMute(stream), audio.getStreamMaxVolume(stream))
        try {
            if (plan.unmute) audio.adjustStreamVolume(stream, AudioManager.ADJUST_UNMUTE, 0)
            plan.steps?.let { audio.setStreamVolume(stream, it, 0) }
            if (plan.mute) audio.adjustStreamVolume(stream, AudioManager.ADJUST_MUTE, 0)
        } catch (_: SecurityException) {
            // Do not disturb may refuse; the observer reports what is true.
        }
        report()
    }
}
