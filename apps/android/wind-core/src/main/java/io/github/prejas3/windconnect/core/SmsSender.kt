package io.github.prejas3.windconnect.core

import android.Manifest
import android.annotation.SuppressLint
import android.app.Activity
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.os.Build
import android.telephony.SmsManager
import android.telephony.SmsMessage
import android.telephony.SubscriptionManager
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import io.github.prejas3.windconnect.bindings.SmsOutgoing
import io.github.prejas3.windconnect.bindings.SmsSim
import java.io.File
import java.util.concurrent.ConcurrentHashMap

/**
 * CCS/1 M:WN-56 sends the PC's messages (PROTOCOL.md §9.11 `Send`): text to one number as (multipart) SMS with sent and
 * delivery reports; a picture or several recipients as MMS (MmsPdu -> FileProvider -> sendMultimediaMessage). States go
 * to [report] by the core's token: 1 sent, 2 delivered, 3 failed. RULE:SEND_SMS checked on every send; as a non-default
 * SMS app Android itself writes what was sent to the provider (so the PC's watch sees it). No text or number is logged.
 */
class SmsSender(context: Context, private val report: (ULong, UByte) -> Unit) {
    private val app = context.applicationContext
    private val outbox = File(app.cacheDir, "mms-out")

    fun canSend(): Boolean = granted(Manifest.permission.SEND_SMS)
    fun canListSims(): Boolean = granted(Manifest.permission.READ_PHONE_STATE)
    private fun granted(p: String) = app.checkSelfPermission(p) == PackageManager.PERMISSION_GRANTED

    /** SIMs (READ_PHONE_STATE), else the default one alone. */
    @SuppressLint("MissingPermission")
    fun sims(): List<SmsSim> {
        val default = SubscriptionManager.getDefaultSmsSubscriptionId()
        if (canListSims()) {
            val list = app.getSystemService(SubscriptionManager::class.java)?.activeSubscriptionInfoList.orEmpty()
            if (list.isNotEmpty()) {
                return list.take(4).map { info ->
                    val label = (info.displayName ?: info.carrierName ?: "SIM ${info.simSlotIndex + 1}").toString()
                    SmsSim(info.subscriptionId.toULong(), SmsRules.clean(label).take(64).ifBlank { "SIM ${info.simSlotIndex + 1}" },
                        info.simSlotIndex.coerceAtLeast(0).toUInt(), info.subscriptionId == default || list.size == 1)
                }
            }
        }
        return listOf(SmsSim(default.coerceAtLeast(0).toULong(), "SIM", 0u, true))
    }

    private fun manager(sim: ULong?): SmsManager {
        val id = sim?.toInt() ?: SubscriptionManager.getDefaultSmsSubscriptionId()
        val base = app.getSystemService(SmsManager::class.java)
        return if (Build.VERSION.SDK_INT >= 31) {
            if (id >= 0) base.createForSubscriptionId(id) else base
        } else {
            @Suppress("DEPRECATION")
            if (id >= 0) SmsManager.getSmsManagerForSubscriptionId(id) else SmsManager.getDefault()
        }
    }

    // CCS/1 FLOW:sent/delivery PendingIntents (explicit, this package) -> receiver -> per-token part counts -> report.
    private class Pending(val parts: Int, val mms: Boolean) {
        var sent = 0
        var delivered = 0
        var failed = false
    }
    private val pending = ConcurrentHashMap<ULong, Pending>()
    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val token = intent.getLongExtra(EXTRA_TOKEN, -1).takeIf { it >= 0 }?.toULong() ?: return
            val state = pending[token] ?: return
            synchronized(state) {
                when (intent.action) {
                    ACTION_SENT -> {
                        if (resultCode != Activity.RESULT_OK) state.failed = true
                        state.sent++
                        if (state.failed && state.sent >= state.parts) finish(token, FAILED)
                        else if (state.sent >= state.parts) {
                            report(token, SENT)
                            if (state.mms) pending.remove(token)
                        }
                    }
                    ACTION_DELIVERED -> {
                        val status = deliveryStatus(intent)
                        if (status == DELIVERY_FAILED) finish(token, FAILED)
                        else if (status == DELIVERY_DONE && ++state.delivered >= state.parts) finish(token, DELIVERED)
                    }
                }
            }
            if (intent.action == ACTION_SENT && intent.getBooleanExtra(EXTRA_MMS, false)) {
                File(outbox, "$token.pdu").delete()
            }
        }
    }

    private fun finish(token: ULong, state: UByte) {
        pending.remove(token)
        report(token, state)
    }

    init {
        ContextCompat.registerReceiver(app, receiver, IntentFilter().apply { addAction(ACTION_SENT); addAction(ACTION_DELIVERED) },
            ContextCompat.RECEIVER_NOT_EXPORTED)
    }

    fun close() = try { app.unregisterReceiver(receiver) } catch (_: IllegalArgumentException) {}

    /** 0 taken (states follow), 1 no SEND_SMS, 4 the system refused it. */
    fun send(token: ULong, message: SmsOutgoing): UByte {
        if (!canSend()) return 1u
        return try {
            val mms = message.image != null || message.to.size > 1
            if (mms) sendMms(token, message) else sendSms(token, message)
            0u
        } catch (_: Exception) {
            pending.remove(token)
            4u
        }
    }

    private fun intent(action: String, token: ULong, part: Int, mms: Boolean, mutable: Boolean): PendingIntent {
        val i = Intent(action).setPackage(app.packageName).putExtra(EXTRA_TOKEN, token.toLong()).putExtra(EXTRA_MMS, mms)
        // The delivery report's PDU is filled in by the system: that intent must be mutable (explicit, so safe).
        val flags = PendingIntent.FLAG_ONE_SHOT or (if (mutable) PendingIntent.FLAG_MUTABLE else PendingIntent.FLAG_IMMUTABLE)
        return PendingIntent.getBroadcast(app, (token.toLong() * 64 + part * 2 + if (mutable) 1 else 0).toInt(), i, flags)
    }

    private fun sendSms(token: ULong, message: SmsOutgoing) {
        val sms = manager(message.sim)
        val parts = sms.divideMessage(message.body)
        pending[token] = Pending(parts.size, false)
        val sent = ArrayList(parts.indices.map { intent(ACTION_SENT, token, it, false, false) })
        val delivered = ArrayList(parts.indices.map { intent(ACTION_DELIVERED, token, it, false, true) })
        sms.sendMultipartTextMessage(message.to.single(), null, parts, sent, delivered)
    }

    private fun sendMms(token: ULong, message: SmsOutgoing) {
        val image = message.image?.let { MmsPdu.Image(message.imageMime ?: "image/jpeg", it) }
        val pdu = MmsPdu.sendRequest(message.to, message.body, image, "T" + java.lang.Long.toHexString(System.currentTimeMillis()) + token)
        outbox.mkdirs()
        val file = File(outbox, "$token.pdu").apply { writeBytes(pdu) }
        val uri = FileProvider.getUriForFile(app, app.packageName + ".mmsout", file)
        pending[token] = Pending(1, true)
        manager(message.sim).sendMultimediaMessage(app, uri, null, null, intent(ACTION_SENT, token, 0, true, false))
    }

    companion object {
        private const val ACTION_SENT = "io.github.prejas3.windconnect.SMS_SENT"
        private const val ACTION_DELIVERED = "io.github.prejas3.windconnect.SMS_DELIVERED"
        private const val EXTRA_TOKEN = "token"
        private const val EXTRA_MMS = "mms"
        private const val SENT: UByte = 1u
        private const val DELIVERED: UByte = 2u
        private const val FAILED: UByte = 3u
        private const val DELIVERY_DONE = 0
        private const val DELIVERY_PENDING = 1
        private const val DELIVERY_FAILED = 2

        /** The report's TP-Status: complete (< 32), still trying (32..63) or failed (>= 64). */
        private fun deliveryStatus(intent: Intent): Int {
            val pdu = intent.getByteArrayExtra("pdu") ?: return DELIVERY_PENDING
            val format = intent.getStringExtra("format") ?: "3gpp"
            val status = SmsMessage.createFromPdu(pdu, format)?.status ?: return DELIVERY_PENDING
            return SmsRules.delivery(status)
        }
    }
}
