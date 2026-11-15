package io.github.prejas3.windconnect

import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import io.github.prejas3.windconnect.bindings.TouchpadCommand
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

class RemoteKeyboardUiTest {
    @get:Rule val compose=createComposeRule()

    @Test fun compositionBoundsAndDuplicateClicksKeepDraftLocal() {
        var draft by mutableStateOf(TextFieldValue("ą",composition=TextRange(0,1)))
        var active by mutableStateOf(true)
        val sent=mutableListOf<TouchpadCommand>()
        compose.setContent {MaterialTheme {RemoteKeyboard(draft,{draft=it},active){sent.add(it);true}}}
        compose.onNodeWithTag("remoteKeyboardSend").assertIsNotEnabled()
        compose.runOnIdle {assertTrue(sent.isEmpty());draft=TextFieldValue("ą".repeat(2049))}
        compose.onNodeWithTag("remoteKeyboardSend").assertIsNotEnabled()
        compose.runOnIdle {draft=TextFieldValue("Zażółć gęślą jaźń 😀")}
        val click=compose.onNodeWithTag("remoteKeyboardSend").assertIsEnabled().fetchSemanticsNode().config[SemanticsActions.OnClick].action!!
        compose.runOnIdle {click();click()}
        compose.runOnIdle {assertEquals(1,sent.size);assertEquals("Zażółć gęślą jaźń 😀",(sent.single() as TouchpadCommand.Text).text);assertEquals("",draft.text);active=false}
        compose.onNodeWithTag("remoteKeyboardSend").assertIsNotEnabled()
    }

    @Test fun failedSubmissionRetainsDraftAndEditingAllowsAnIdenticalNextSubmission() {
        var draft by mutableStateOf(TextFieldValue("ą"))
        var allow=false
        val sent=mutableListOf<TouchpadCommand>()
        compose.setContent {MaterialTheme {RemoteKeyboard(draft,{draft=it},true){if(allow){sent.add(it);true}else false}}}
        compose.onNodeWithTag("remoteKeyboardSend").performClick()
        compose.runOnIdle {assertEquals("ą",draft.text);assertTrue(sent.isEmpty());allow=true}
        compose.onNodeWithTag("remoteKeyboardSend").performClick()
        compose.onNodeWithTag("remoteKeyboardDraft").performTextInput("ą")
        compose.onNodeWithTag("remoteKeyboardSend").performClick()
        compose.runOnIdle {assertEquals(2,sent.size)}
    }
}
