package io.github.prejas3.windconnect.core

import android.provider.Telephony

/** WN-55: how Android's SMS/MMS rows map to PROTOCOL.md §9.11 (folders) and what text goes to the PC. Pure, unit-tested. */
object SmsRules {
    fun smsFolder(type: Int): UByte = when (type) {
        Telephony.Sms.MESSAGE_TYPE_SENT -> 1u
        Telephony.Sms.MESSAGE_TYPE_OUTBOX, Telephony.Sms.MESSAGE_TYPE_QUEUED -> 2u
        Telephony.Sms.MESSAGE_TYPE_FAILED -> 3u
        else -> 0u
    }

    fun mmsFolder(box: Int): UByte = when (box) {
        Telephony.Mms.MESSAGE_BOX_SENT -> 1u
        Telephony.Mms.MESSAGE_BOX_OUTBOX -> 2u
        Telephony.Mms.MESSAGE_BOX_FAILED -> 3u
        else -> 0u
    }

    /** A delivery report's TP-Status (or the provider's `status`): 0 done (< 32), 1 still trying (32..63), 2 failed. */
    fun delivery(status: Int): Int = when {
        status < 32 -> 0
        status < 64 -> 1
        else -> 2
    }

    /** The provider's `status` of a sent SMS as PROTOCOL.md §9.11 `delivery` (0 pending, 1 delivered, 2 failed); -1: none asked. */
    fun providerDelivery(status: Int): UByte? = when {
        status < 0 -> null
        else -> when (delivery(status)) { 0 -> 1u; 1 -> 0u; else -> 2u }
    }

    /** Control and bidirectional-override characters out (the PC shows plain text). */
    fun clean(text: String): String = text.filterNot { c ->
        (c.isISOControl() && c != '\n') || c in '\u202A'..'\u202E' || c in '\u2066'..'\u2069' || c == '\u200E' || c == '\u200F'
    }
}
