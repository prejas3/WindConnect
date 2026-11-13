package io.github.prejas3.windconnect

import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import io.github.prejas3.windconnect.core.PcVolume
import io.github.prejas3.windconnect.ui.ThemeMode
import io.github.prejas3.windconnect.ui.WindConnectTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

/** WN-48: the PC's volume on the phone: no bounce-back from an echoed level, then the pushed level wins; mute only mutes. */
class PcVolumeTest {
    @get:Rule val compose = createComposeRule()

    private fun valueIs(text: String) = compose.onNodeWithTag("pcVolumeValue").assert(SemanticsMatcher.expectValue(SemanticsProperties.Text, listOf(androidx.compose.ui.text.AnnotatedString(text))))

    @Test fun anEchoDoesNotMoveItBackThenThePushedLevelWins() {
        var volume by mutableStateOf(PcVolume(40, false))
        val levels = mutableListOf<Int>()
        val mutes = mutableListOf<Boolean>()
        compose.mainClock.autoAdvance = true
        compose.setContent {
            WindConnectTheme(ThemeMode.Light, false) { Column { PcVolumeControls(volume, { levels += it }, { mutes += it }) } }
        }
        valueIs("40%")
        // A local change (as TalkBack or a drag would set it).
        compose.onNodeWithTag("pcVolumeSlider").performSemanticsAction(SemanticsActions.SetProgress) { it(0.7f) }
        assertEquals(70, levels.last())
        // The echo of an older level within 500 ms: ignored.
        volume = PcVolume(45, false)
        compose.waitForIdle()
        valueIs("70%")
        // After the hold, the pushed level wins.
        Thread.sleep(VOLUME_HOLD_MS + 200)
        compose.waitUntil(2_000) { runCatching { valueIs("45%") }.isSuccess }
        // Mute sends only muted.
        compose.onNodeWithText("Wycisz komputer").performClick()
        assertEquals(listOf(true), mutes)
        assertEquals(1, levels.size)
        valueIs("Wyciszony")
    }
}
