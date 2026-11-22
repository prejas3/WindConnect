package io.github.prejas3.windconnect

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * WN-158: the S25 (One UI 7) closed the SEND_SMS request in ~20 ms without a dialog and without a user refusal; the app
 * showed it as the person's refusal, with no way forward that worked.
 */
class SmsSendAskTest {
    @Test fun neverAskedYetAsksAndroidNotTheSettings() {
        assertEquals(SmsSendAsk.Step.AskSystem, SmsSendAsk.onTurn(on = true, granted = false))
        assertEquals(SmsSendAsk.Step.Save, SmsSendAsk.onTurn(on = true, granted = true))
        assertEquals(SmsSendAsk.Step.Save, SmsSendAsk.onTurn(on = false, granted = false))
    }

    @Test fun aDenialWithoutADialogIsASystemBlockNotARefusal() {
        // As on the S25: denied after ~20 ms, no rationale (nothing was shown).
        assertEquals(SmsSendAsk.Outcome.Blocked, SmsSendAsk.outcome(granted = false, rationale = false, elapsedMs = 20))
        // A person who read the dialog and tapped "Don't allow": slower, and Android offers a rationale next time.
        assertEquals(SmsSendAsk.Outcome.Refused, SmsSendAsk.outcome(granted = false, rationale = true, elapsedMs = 2_500))
        // "Don't allow" twice: no rationale any more, but it took a person's time.
        assertEquals(SmsSendAsk.Outcome.Refused, SmsSendAsk.outcome(granted = false, rationale = false, elapsedMs = 1_800))
        assertEquals(SmsSendAsk.Outcome.Granted, SmsSendAsk.outcome(granted = true, rationale = false, elapsedMs = 15))
    }

    @Test fun theBlockedRowSaysAndroidClosedItNotThePerson() {
        StringsForTests.useEnglish()
        val text = str(R.string.sms_send_blocked)
        assertTrue(text, text.contains("without asking") && text.contains("SMS") && !text.contains("refus"))
    }
}
