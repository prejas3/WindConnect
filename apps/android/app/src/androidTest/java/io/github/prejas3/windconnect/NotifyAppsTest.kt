package io.github.prejas3.windconnect

import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import io.github.prejas3.windconnect.core.NotifyAppMode
import io.github.prejas3.windconnect.core.NotifyAppsState
import io.github.prejas3.windconnect.ui.ThemeMode
import io.github.prejas3.windconnect.ui.WindConnectTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

/** WN-45: the app filter's list: modes, switches named by the app, the "nothing goes" line, search. */
class NotifyAppsTest {
    @get:Rule val compose = createComposeRule()
    private val labels = mapOf("org.thoughtcrime.securesms" to "Signal", "pl.bank" to "Bank", "com.whatsapp" to "WhatsApp")

    @Test fun switchesFollowTheModeAndSayWhenNothingGoes() {
        var state by mutableStateOf(NotifyAppsState(seen = labels.keys.toList(), blocked = setOf("pl.bank")))
        val calls = mutableListOf<String>()
        compose.setContent {
            WindConnectTheme(ThemeMode.Light, false) {
                Column {
                    NotifyAppsList(state, { labels.getValue(it) }, { calls += "mode $it"; state = state.copy(mode = it) },
                        { pkg, on -> calls += "$pkg $on" })
                }
            }
        }
        // TalkBack: each switch is named by the app's label.
        compose.onNodeWithText("Signal").assertIsOn()
        compose.onNodeWithText("Bank").assertIsOff()
        compose.onNodeWithText("Bank").performClick()
        assertEquals("pl.bank true", calls.last())
        // "Tylko wybrane" with nothing chosen: everything off, and the line says nothing goes.
        compose.onNodeWithTag("notifyAppMode${NotifyAppMode.OnlyChosen}").performClick()
        assertEquals("mode OnlyChosen", calls.last())
        compose.onNodeWithText("Signal").assertIsOff()
        compose.onNodeWithText("Nie wysyłasz żadnych powiadomień. Wybierz aplikacje poniżej.").assertExists()
        state = state.copy(chosen = setOf("com.whatsapp"))
        compose.onNodeWithText("WhatsApp").assertIsOn()
        compose.onNodeWithText("Nie wysyłasz żadnych powiadomień. Wybierz aplikacje poniżej.").assertDoesNotExist()
    }

    @Test fun searchOnceTheListIsLong() {
        val many = (1..14).map { "app.n$it" }
        compose.setContent {
            WindConnectTheme(ThemeMode.Light, false) {
                Column { NotifyAppsList(NotifyAppsState(seen = many), { "Aplikacja ${it.removePrefix("app.n")}" }, {}, { _, _ -> }) }
            }
        }
        compose.onNodeWithTag("notifyAppSearch").performTextInput("14")
        compose.onNodeWithText("Aplikacja 14").assertExists()
        compose.onNodeWithText("Aplikacja 2").assertDoesNotExist()
    }
}
