package io.github.prejas3.windconnect.core

import android.content.Context
import android.media.AudioManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.telephony.PhoneStateListener
import android.telephony.TelephonyManager
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** WN-48: the phone's call state as the PC gets it (PROTOCOL.md §9.10): 0 idle, 1 ringing, 2 offhook. */
enum class CallState(val code: Int) { Idle(0), Ringing(1), Offhook(2) }

/**
 * CCS/1 RULE:WN-48 the call state from what Android gives without a permission: the audio mode (API 31+), the telephony state
 * (API 29-30, no number), and a call notification seen by [NotifyListenerService] (for vibrate/silent, where some phones never
 * enter MODE_RINGTONE; only while "Powiadomienia na komputerze" has its access). Pure; the watcher feeds it.
 */
internal object CallStates {
    fun fromMode(mode: Int): CallState = when (mode) {
        AudioManager.MODE_RINGTONE -> CallState.Ringing
        AudioManager.MODE_IN_CALL, AudioManager.MODE_IN_COMMUNICATION, AudioManager.MODE_CALL_SCREENING -> CallState.Offhook
        else -> CallState.Idle
    }

    fun fromTelephony(state: Int): CallState = when (state) {
        TelephonyManager.CALL_STATE_RINGING -> CallState.Ringing
        TelephonyManager.CALL_STATE_OFFHOOK -> CallState.Offhook
        else -> CallState.Idle
    }

    /** The system's state and whether a call notification is up: a notification alone (vibrate) counts as ringing. */
    fun combine(system: CallState, callNotification: Boolean): CallState =
        if (system == CallState.Idle && callNotification) CallState.Ringing else system
}

/** WN-48: call notifications seen by the listener (keys only), a permission-free second signal. */
internal object CallNotifications {
    private val keys = MutableStateFlow<Set<String>>(emptySet())
    val active: StateFlow<Set<String>> = keys.asStateFlow()
    fun posted(key: String) { keys.value = keys.value + key }
    fun removed(key: String) { if (key in keys.value) keys.value = keys.value - key }
}

/**
 * Watches the call state while a session runs; [onState] gets every change. Ringing and offhook go at once (the PC must pause
 * before the second ring); idle only after [IDLE_AFTER_MS] of steady idle, so a flicker doesn't resume the PC's music. No polling.
 */
internal class CallWatcher(context: Context, private val onState: (CallState) -> Unit) {
    private val app = context.applicationContext
    private val audio = app.getSystemService(AudioManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    @Volatile private var system = CallState.Idle
    @Volatile private var notification = false
    @Volatile private var reported: CallState? = null
    private val idle = Runnable { report(CallState.Idle) }
    private var modeListener: AudioManager.OnModeChangedListener? = null
    @Suppress("DEPRECATION") private var phoneListener: PhoneStateListener? = null

    fun start() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            val listener = AudioManager.OnModeChangedListener { mode -> system = CallStates.fromMode(mode); update() }
            audio.addOnModeChangedListener(app.mainExecutor, listener)
            modeListener = listener
            system = CallStates.fromMode(audio.mode)
        } else {
            @Suppress("DEPRECATION")
            val listener = object : PhoneStateListener(app.mainExecutor) {
                @Deprecated("API 29-30 only") override fun onCallStateChanged(state: Int, number: String?) {
                    system = CallStates.fromTelephony(state); update()
                }
            }
            @Suppress("DEPRECATION")
            app.getSystemService(TelephonyManager::class.java).listen(listener, PhoneStateListener.LISTEN_CALL_STATE)
            phoneListener = listener
        }
        notification = CallNotifications.active.value.isNotEmpty()
        // The first state goes out at once, idle included (the PC settles a pause from a dropped session).
        report(CallStates.combine(system, notification))
    }

    /** The listener's call notifications changed. */
    fun notifications(present: Boolean) { notification = present; update() }

    private fun update() {
        val now = CallStates.combine(system, notification)
        main.removeCallbacks(idle)
        if (now == CallState.Idle) main.postDelayed(idle, IDLE_AFTER_MS) else report(now)
    }

    @Synchronized private fun report(state: CallState) {
        if (reported == state) return
        reported = state
        onState(state)
    }

    fun stop() {
        main.removeCallbacks(idle)
        modeListener?.let { if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) audio.removeOnModeChangedListener(it) }
        modeListener = null
        @Suppress("DEPRECATION")
        phoneListener?.let { app.getSystemService(TelephonyManager::class.java).listen(it, PhoneStateListener.LISTEN_NONE) }
        phoneListener = null
    }

    companion object { const val IDLE_AFTER_MS = 1_500L }
}
