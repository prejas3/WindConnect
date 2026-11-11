package io.github.prejas3.windconnect

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import io.github.prejas3.windconnect.ui.ThemeMode
import io.github.prejas3.windconnect.ui.WindConnectTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

/** WC-34: the Welcome card on first run and the Gotowe screen's background decision. */
class OnboardingTest {
    @get:Rule val compose = createComposeRule()

    @Test fun welcomeExplainsBeforeAskingAndMatchesTheComputersWording() {
        var scanned = false
        compose.setContent { WindConnectTheme(ThemeMode.Light, false) {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                HomeScreen(PairingUi(ready = true, initialized = true), { scanned = true }, onDiagnostics = {}, onNearby = {})
            }
        } }
        // TalkBack: the card starts with a heading; the camera note precedes the system prompt.
        compose.onNode(hasText("Witaj w WindConnect") and SemanticsMatcher.keyIsDefined(SemanticsProperties.Heading)).assertExists()
        compose.onNodeWithTag("cameraNote").assertTextContains("obraz nie opuszcza telefonu", substring = true)
        compose.onNodeWithTag("scanQr").assertTextEquals("Zeskanuj kod z komputera").performClick()
        assertTrue(scanned)
        compose.onNodeWithTag("pairNearby").assertExists()
        // The Welcome card is the first thing on the screen.
        val welcomeTop = compose.onNodeWithTag("welcome").fetchSemanticsNode().boundsInRoot.top
        val stripTop = compose.onNodeWithTag("connectionStrip").fetchSemanticsNode().boundsInRoot.top
        assertTrue("welcome above the strip", welcomeTop < stripTop)
    }

    @Test fun doneScreenAsksOnceAndSaysTheConsequence() {
        var done = false
        compose.setContent { WindConnectTheme(ThemeMode.Dark, false) {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                DoneScreen("Biurko", "Połączono z komputerem") { done = true }
            }
        } }
        compose.onNode(hasText("Gotowe") and SemanticsMatcher.keyIsDefined(SemanticsProperties.Heading)).assertExists()
        compose.onNodeWithTag("backgroundLater").performScrollTo().performClick()
        compose.onNodeWithTag("backgroundOutcome").assertTextContains("później w Ustawieniach", substring = true)
        compose.onAllNodesWithTag("keepBackground").assertCountEquals(0)
        compose.onNodeWithTag("pairingDone").performScrollTo().performClick()
        assertTrue(done)
    }

    @Test fun backgroundOutcomeSentences() {
        assertNull(backgroundOutcome(BackgroundChoice()))
        assertTrue(backgroundOutcome(BackgroundChoice(decided = true, keep = true))!!.startsWith("Włączone"))
        assertEquals(
            "Połączenie działa w tle, ale bez powiadomień nie zobaczysz stanu komputera ani akcji „Wyślij schowek”. Zmienisz to w Ustawieniach.",
            backgroundOutcome(BackgroundChoice(decided = true, keep = true, notificationsDenied = true)),
        )
        assertTrue(backgroundOutcome(BackgroundChoice(decided = true, keep = false))!!.contains("później"))
    }
}
