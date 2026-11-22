package io.github.prejas3.windconnect

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import org.junit.Assert.*
import org.junit.Test

class RemoteKeyboardDraftTest {
    init { StringsForTests.useEnglish() }
    @Test fun onlyExplicitCompleteAndBoundedTextCanBeSubmitted() {
        assertTrue(RemoteKeyboardDraft.canSend(TextFieldValue("Zażółć gęślą jaźń 😀"))) // i18n: data
        assertFalse(RemoteKeyboardDraft.canSend(TextFieldValue("ą",composition=TextRange(0,1)))) // i18n: data
        assertFalse(RemoteKeyboardDraft.canSend(TextFieldValue()))
        assertTrue(RemoteKeyboardDraft.canSend(TextFieldValue("ą".repeat(2048)))) // i18n: data
        assertFalse(RemoteKeyboardDraft.canSend(TextFieldValue("ą".repeat(2049)))) // i18n: data
        assertFalse(RemoteKeyboardDraft.canSend(TextFieldValue("nul\u0000text")))
        assertFalse(RemoteKeyboardDraft.canSend(TextFieldValue("\uD83D")))
    }
}
