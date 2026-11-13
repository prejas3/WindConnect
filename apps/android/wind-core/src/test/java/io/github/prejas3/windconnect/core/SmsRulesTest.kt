package io.github.prejas3.windconnect.core

import android.provider.Telephony
import org.junit.Assert.assertEquals
import org.junit.Test

class SmsRulesTest {
    @Test fun smsTypesMapToFolders() {
        assertEquals(0.toUByte(), SmsRules.smsFolder(Telephony.Sms.MESSAGE_TYPE_INBOX))
        assertEquals(1.toUByte(), SmsRules.smsFolder(Telephony.Sms.MESSAGE_TYPE_SENT))
        assertEquals(2.toUByte(), SmsRules.smsFolder(Telephony.Sms.MESSAGE_TYPE_OUTBOX))
        assertEquals(2.toUByte(), SmsRules.smsFolder(Telephony.Sms.MESSAGE_TYPE_QUEUED))
        assertEquals(3.toUByte(), SmsRules.smsFolder(Telephony.Sms.MESSAGE_TYPE_FAILED))
        assertEquals(0.toUByte(), SmsRules.smsFolder(99))
    }

    @Test fun mmsBoxesMapToFolders() {
        assertEquals(0.toUByte(), SmsRules.mmsFolder(Telephony.Mms.MESSAGE_BOX_INBOX))
        assertEquals(1.toUByte(), SmsRules.mmsFolder(Telephony.Mms.MESSAGE_BOX_SENT))
        assertEquals(2.toUByte(), SmsRules.mmsFolder(Telephony.Mms.MESSAGE_BOX_OUTBOX))
        assertEquals(3.toUByte(), SmsRules.mmsFolder(Telephony.Mms.MESSAGE_BOX_FAILED))
    }

    @Test fun cleanDropsControlAndBidiButKeepsText() {
        assertEquals("ab\nc", SmsRules.clean("a\u202Eb\u0007\nc\u200F"))
        assertEquals("Zażółć 👋", SmsRules.clean("Zażółć 👋"))
        assertEquals("x", SmsRules.clean("\u2066x\u2069"))
    }
}
