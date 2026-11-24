package io.github.prejas3.windconnect.core

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioTrack
import android.os.Build
import android.os.SystemClock
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager
import io.github.prejas3.windconnect.bindings.FindSink
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.lang.ref.WeakReference
import kotlin.math.PI
import kotlin.math.min
import kotlin.math.sin

/**
 * CCS/1 M:WN-50 the phone rings for the PC (PROTOCOL.md §9.5).
 * RULE:a generated tone on the alarm stream (plays in silent and vibrate mode; no file, no prepare, never "None"); `ring`
 * returns only once the track's head moved, so the PC's "ringing" is an upper bound of the real start. No WAKE_LOCK:
 * audioserver holds its own while playing. LIMIT:DND that blocks alarms mutes it (bypassing needs policy access).
 * Stop: the notification (own high-importance silent channel), the app's bar, volume keys in the app.
 */
internal class PhoneRinger(context: Context, preferences: SharedPreferences, private val peerName: () -> String?) : FindSink {
    private val context = context.applicationContext
    private val audio = this.context.getSystemService(AudioManager::class.java)
    private val volume = AlarmVolume(
        object : AlarmVolume.Store {
            override fun get(key: String) = if (preferences.contains(key)) preferences.getInt(key, 0) else null
            override fun put(key: String, value: Int) { preferences.edit().putInt(key, value).commit() }
            override fun remove(key: String) { preferences.edit().remove(key).commit() }
        },
        object : AlarmVolume.Volume {
            override val current get() = audio.getStreamVolume(AudioManager.STREAM_ALARM)
            override val max get() = audio.getStreamMaxVolume(AudioManager.STREAM_ALARM)
            override fun set(value: Int) {
                // DND (total silence) may refuse a change; the ring still plays at the person's level.
                try { audio.setStreamVolume(AudioManager.STREAM_ALARM, value, 0) } catch (_: SecurityException) {}
            }
        },
    )
    private val mutable = MutableStateFlow(false)
    val ringing: StateFlow<Boolean> = mutable.asStateFlow()
    private var track: AudioTrack? = null
    /** Asks the core to stop (the PC gets state 1); set by WindCore. */
    @Volatile var stopRequested: () -> Unit = {}

    init { current = WeakReference(this) }

    /** A volume left at max by a process that died while ringing. */
    fun recover() = synchronized(this) { if (track == null) volume.restore() }

    @Synchronized override fun ring(): Boolean {
        if (track != null) return true
        volume.raise()
        val pcm = RingTone.pcm()
        val created = try {
            AudioTrack.Builder()
                .setAudioAttributes(ATTRIBUTES)
                .setAudioFormat(AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_16BIT).setSampleRate(RingTone.RATE).setChannelMask(AudioFormat.CHANNEL_OUT_MONO).build())
                .setTransferMode(AudioTrack.MODE_STATIC)
                .setBufferSizeInBytes(pcm.size * 2)
                .build()
        } catch (_: Exception) { null }
        // A static track is STATE_NO_STATIC_DATA until written; only an uninitialized one is broken.
        if (created == null || created.state == AudioTrack.STATE_UNINITIALIZED) {
            created?.release(); volume.restore(); return false
        }
        val playing = try {
            created.write(pcm, 0, pcm.size)
            created.setLoopPoints(0, pcm.size, -1)
            created.play()
            val deadline = SystemClock.elapsedRealtime() + START_WAIT_MS
            while (created.playbackHeadPosition == 0 && SystemClock.elapsedRealtime() < deadline) Thread.sleep(2)
            created.playbackHeadPosition > 0
        } catch (_: Exception) { false }
        if (!playing) { created.release(); volume.restore(); return false }
        track = created
        vibrator()?.let { v -> try { @Suppress("DEPRECATION") v.vibrate(VibrationEffect.createWaveform(longArrayOf(0, 500, 700), 0), ATTRIBUTES) } catch (_: Exception) {} }
        showNotification()
        mutable.value = true
        return true
    }

    @Synchronized override fun stop() {
        val playing = track ?: return
        track = null
        try { playing.stop() } catch (_: Exception) {}
        playing.release()
        vibrator()?.cancel()
        volume.restore()
        context.getSystemService(NotificationManager::class.java).cancel(NOTIFICATION)
        mutable.value = false
    }

    private fun vibrator(): Vibrator? =
        if (Build.VERSION.SDK_INT >= 31) context.getSystemService(VibratorManager::class.java)?.defaultVibrator
        else @Suppress("DEPRECATION") context.getSystemService(Vibrator::class.java)

    private fun showNotification() {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(CHANNEL, context.getString(R.string.find_channel), NotificationManager.IMPORTANCE_HIGH).apply {
            description = context.getString(R.string.find_channel_description)
            setSound(null, null)
            enableVibration(false)
            lockscreenVisibility = Notification.VISIBILITY_PUBLIC
        })
        val stop = PendingIntent.getBroadcast(context, 50, Intent(context, FindStopReceiver::class.java), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val icon = context.resources.getIdentifier("ic_connection", "drawable", context.packageName).takeIf { it != 0 } ?: android.R.drawable.ic_lock_idle_alarm
        val text = peerName()?.let { context.getString(R.string.find_ringing_for, it) } ?: context.getString(R.string.find_ringing)
        val notification = Notification.Builder(context, CHANNEL)
            .setSmallIcon(icon)
            .setContentTitle(context.getString(R.string.find_channel))
            .setContentText(text)
            .setCategory(Notification.CATEGORY_ALARM)
            .setOngoing(true)
            .setVisibility(Notification.VISIBILITY_PUBLIC)
            .setContentIntent(stop)
            .addAction(Notification.Action.Builder(null, context.getString(R.string.find_stop_action), stop).build())
            .build()
        // Without POST_NOTIFICATIONS nothing shows; the app's bar and volume keys still stop it (WN-50 review).
        try { manager.notify(NOTIFICATION, notification) } catch (_: SecurityException) {}
    }

    companion object {
        const val CHANNEL = "find_phone"
        const val NOTIFICATION = 50
        private const val START_WAIT_MS = 400L
        private val ATTRIBUTES: AudioAttributes = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_ALARM)
            .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
            .build()
        @Volatile private var current: WeakReference<PhoneRinger>? = null

        /** The notification's stop: the live ringer, if any. */
        internal fun stopFromSystem() { current?.get()?.stopRequested?.invoke() }

    }
}

/** The ring notification's "Stop" (`find_stop_action`; not exported; only our PendingIntent reaches it). */
class FindStopReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) = PhoneRinger.stopFromSystem()
}

/** The ring's sound, pure so it can be tested off-device. */
internal object RingTone {
    const val RATE = 44_100

    /** One 1.2 s period: two short beeps (880 Hz, 1320 Hz) and a pause; 5 ms ramps avoid clicks. */
    fun pcm(): ShortArray {
        val samples = ShortArray(RATE * 12 / 10)
        fun beep(startMs: Int, lengthMs: Int, hz: Double) {
            val start = RATE * startMs / 1000
            val length = RATE * lengthMs / 1000
            val ramp = RATE * 5 / 1000
            for (i in 0 until length) {
                val edge = min(1.0, min(i, length - 1 - i).toDouble() / ramp)
                samples[start + i] = (sin(2 * PI * hz * i / RATE) * edge * 0.8 * Short.MAX_VALUE).toInt().toShort()
            }
        }
        beep(0, 180, 880.0)
        beep(230, 180, 1320.0)
        return samples
    }
}
