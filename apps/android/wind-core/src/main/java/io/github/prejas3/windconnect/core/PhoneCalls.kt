package io.github.prejas3.windconnect.core

import android.Manifest
import android.annotation.SuppressLint
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.media.AudioManager
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.provider.CallLog
import android.provider.ContactsContract
import android.telecom.TelecomManager
import android.telephony.TelephonyManager
import androidx.core.content.ContextCompat
import io.github.prejas3.windconnect.bindings.CallActionSink
import io.github.prejas3.windconnect.bindings.CallerInfo
import io.github.prejas3.windconnect.bindings.MissedCall
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * CCS/1 M:WN-57 who calls and the missed calls, for the PC (PROTOCOL.md §9.10, §9.12), with "Połączenia na komputerze".
 * FLOW:PHONE_STATE broadcast (the number comes only with READ_CALL_LOG) -> caller (contact name with READ_CONTACTS) -> the
 * session's State; a ring that ends without offhook -> the call log's missed list again. The PC's Reject (TelecomManager.endCall,
 * ANSWER_PHONE_CALLS) and Silence (the ring stream muted, unmuted when the call ends) act only on the ringing call they name.
 * RULE:no number or name is logged; nothing is stored beyond what Android's call log already has.
 */
class PhoneCalls(context: Context) : CallActionSink {
    private val app = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    private val audio = app.getSystemService(AudioManager::class.java)

    fun canRead(): Boolean = granted(Manifest.permission.READ_PHONE_STATE) && granted(Manifest.permission.READ_CALL_LOG)
    fun canReject(): Boolean = granted(Manifest.permission.ANSWER_PHONE_CALLS)
    private fun granted(p: String) = app.checkSelfPermission(p) == PackageManager.PERMISSION_GRANTED

    private val mutableCaller = MutableStateFlow<CallerInfo?>(null)
    /** The ringing or ongoing call's caller, or null. */
    val caller: StateFlow<CallerInfo?> = mutableCaller.asStateFlow()
    private val mutableMissed = MutableStateFlow<List<MissedCall>>(emptyList())
    /** The newest missed calls (7 days, at most 20), newest first. */
    val missed: StateFlow<List<MissedCall>> = mutableMissed.asStateFlow()

    @Volatile private var nextCall = 1L
    @Volatile private var ringing = false
    @Volatile private var answered = false
    @Volatile private var mutedRing = false

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action != TelephonyManager.ACTION_PHONE_STATE_CHANGED) return
            @Suppress("DEPRECATION")
            val number = intent.getStringExtra(TelephonyManager.EXTRA_INCOMING_NUMBER)
            when (intent.getStringExtra(TelephonyManager.EXTRA_STATE)) {
                TelephonyManager.EXTRA_STATE_RINGING -> ringingWith(number)
                TelephonyManager.EXTRA_STATE_OFFHOOK -> { answered = ringing || answered; ringing = false; refreshActions() }
                TelephonyManager.EXTRA_STATE_IDLE -> ended()
            }
        }
    }

    // The broadcast comes twice per change (without and with the number): one call id per ring.
    private fun ringingWith(number: String?) {
        if (!ringing) { ringing = true; answered = false; nextCall++ }
        val current = mutableCaller.value
        if (current != null && current.call.toLong() == nextCall && (number.isNullOrEmpty() || current.number != null)) return
        val clean = number?.takeIf { it.isNotEmpty() }
        mutableCaller.value = CallerInfo(nextCall.toULong(), clean?.let { SmsRules.clean(it).take(64) }, clean?.let(::contactName), actions())
    }

    private fun actions(): UByte = ((if (ringing && canReject()) 1 else 0) or (if (ringing) 2 else 0)).toUByte()

    private fun refreshActions() {
        mutableCaller.value?.let { mutableCaller.value = it.copy(actions = actions()) }
    }

    private fun ended() {
        val missedNow = ringing && !answered
        ringing = false; answered = false
        mutableCaller.value = null
        if (mutedRing) {
            mutedRing = false
            try { audio.adjustStreamVolume(AudioManager.STREAM_RING, AudioManager.ADJUST_UNMUTE, 0) } catch (_: SecurityException) {}
        }
        // The call log is written a moment after the state goes idle.
        if (missedNow) main.postDelayed({ refreshMissed() }, 1_500)
    }

    private fun contactName(number: String): String? {
        if (!granted(Manifest.permission.READ_CONTACTS)) return null
        return try {
            app.contentResolver.query(Uri.withAppendedPath(ContactsContract.PhoneLookup.CONTENT_FILTER_URI, Uri.encode(number)),
                arrayOf(ContactsContract.PhoneLookup.DISPLAY_NAME), null, null, null)?.use { c ->
                if (c.moveToFirst()) c.getString(0)?.let(SmsRules::clean)?.take(128)?.takeIf { it.isNotBlank() } else null
            }
        } catch (_: Exception) { null }
    }

    /** Re-reads the missed calls (READ_CALL_LOG). */
    fun refreshMissed() {
        if (!canRead()) { mutableMissed.value = emptyList(); return }
        val since = System.currentTimeMillis() - 7L * 24 * 60 * 60 * 1000
        val out = ArrayList<MissedCall>()
        try {
            app.contentResolver.query(CallLog.Calls.CONTENT_URI,
                arrayOf(CallLog.Calls.NUMBER, CallLog.Calls.CACHED_NAME, CallLog.Calls.DATE),
                "${CallLog.Calls.TYPE}=? AND ${CallLog.Calls.DATE}>=?",
                arrayOf(CallLog.Calls.MISSED_TYPE.toString(), since.toString()), "${CallLog.Calls.DATE} DESC")?.use { c ->
                while (out.size < 20 && c.moveToNext()) {
                    val number = c.getString(0)?.takeIf { it.isNotBlank() }?.let { SmsRules.clean(it).take(64) } ?: continue
                    val name = c.getString(1)?.let(SmsRules::clean)?.take(128)?.takeIf { it.isNotBlank() } ?: contactName(number)
                    out += MissedCall(number, name, c.getLong(2).coerceAtLeast(0).toULong())
                }
            }
        } catch (_: SecurityException) {}
        mutableMissed.value = out
    }

    private var started = false

    /** Watches calls while "Połączenia na komputerze" is on with its permissions. */
    @Synchronized fun watch(on: Boolean) {
        if (on == started) { if (on) refreshMissed(); return }
        started = on
        if (on) {
            ContextCompat.registerReceiver(app, receiver, IntentFilter(TelephonyManager.ACTION_PHONE_STATE_CHANGED), ContextCompat.RECEIVER_NOT_EXPORTED)
            refreshMissed()
        } else {
            try { app.unregisterReceiver(receiver) } catch (_: IllegalArgumentException) {}
            ringing = false; answered = false
            mutableCaller.value = null
            mutableMissed.value = emptyList()
        }
    }

    private fun current(call: ULong) = ringing && mutableCaller.value?.call == call

    @SuppressLint("MissingPermission")
    override fun reject(call: ULong) {
        if (!current(call) || !canReject()) return
        try {
            @Suppress("DEPRECATION")
            app.getSystemService(TelecomManager::class.java)?.endCall()
        } catch (_: SecurityException) {}
    }

    override fun silence(call: ULong) {
        if (!current(call)) return
        try {
            app.getSystemService(TelecomManager::class.java)?.silenceRinger()
            return
        } catch (_: SecurityException) {
            // Only the default dialer may: mute the ring stream for this call instead.
        }
        try {
            audio.adjustStreamVolume(AudioManager.STREAM_RING, AudioManager.ADJUST_MUTE, 0)
            mutedRing = true
        } catch (_: SecurityException) {}
        refreshActions()
    }
}
