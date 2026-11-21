package io.github.prejas3.windconnect

import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import io.github.prejas3.windconnect.ui.ButtonKind
import io.github.prejas3.windconnect.ui.ThemeMode
import io.github.prejas3.windconnect.ui.WindButton
import io.github.prejas3.windconnect.ui.WindConnectTheme
import io.github.prejas3.windconnect.ui.WindField
import io.github.prejas3.windconnect.ui.WindIconButton
import io.github.prejas3.windconnect.ui.WindSegmented
import io.github.prejas3.windconnect.ui.WindSlider
import io.github.prejas3.windconnect.ui.WindSwitch
import io.github.prejas3.windconnect.ui.WindTextArea
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

/** WN-168: the own M3 controls (slider, switch, buttons, connected group, toggle icon button, fields) behave as specified. */
class M3ControlsTest {
    @get:Rule val compose = createComposeRule()

    @Test fun controlsWorkAndKeepTheirTargets() {
        var clicks = 0
        var on by mutableStateOf(false)
        var limit by mutableStateOf(5)
        var pinned by mutableStateOf(false)
        var name by mutableStateOf("")
        var level by mutableFloatStateOf(.4f)
        compose.setContent {
            WindConnectTheme(mode = ThemeMode.Light, dynamicColor = false) {
                Column {
                    WindButton("Tonal", { clicks++ }, Modifier.testTag("tonal"), kind = ButtonKind.Tonal)
                    WindSwitch("Przełącznik", on, { on = it }, Modifier.testTag("switch"))
                    WindSegmented(listOf(5, 16, 32), limit, { "$it MB" }, { limit = it }, tag = { "seg$it" })
                    WindIconButton("pin", "Przypnij", { pinned = !pinned }, Modifier.testTag("pin"), selected = pinned)
                    WindField(name, { name = it }, "Nazwa", Modifier.testTag("name"), maxLength = 64, errorText = if (name.isEmpty()) "Wpisz nazwę" else null)
                    WindSlider(level, { level = it }, Modifier.testTag("volume"), icon = "volume", valueLabel = { "${(it * 100).toInt()}%" })
                    WindTextArea(androidx.compose.ui.text.input.TextFieldValue("x"), {}, "Klawiatura", Modifier.testTag("area"), enabled = false)
                }
            }
        }
        // Button: the whole 48 dp area takes the tap (the pill inside is 40 dp).
        compose.onNodeWithTag("tonal").assertHeightIsAtLeast(48.dp).performClick()
        compose.runOnIdle { assertEquals(1, clicks) }

        compose.onNodeWithTag("switch").assertIsOff().performClick().assertIsOn()

        // Connected group: one chosen at a time, each segment 48 dp tall.
        compose.onNodeWithTag("seg5").assertIsSelected()
        compose.onNodeWithTag("seg32").assertHeightIsAtLeast(48.dp).performClick()
        compose.onNodeWithTag("seg32").assertIsSelected()
        compose.onNodeWithTag("seg5").assertIsNotSelected()

        // Toggle icon button: its state is read out as selected.
        compose.onNodeWithTag("pin").assertIsNotSelected().assertHeightIsAtLeast(48.dp).performClick()
        compose.onNodeWithTag("pin").assertIsSelected()

        // Field: the error is announced, the input is cut at the limit and counted.
        compose.onNodeWithTag("name").assert(SemanticsMatcher.keyIsDefined(androidx.compose.ui.semantics.SemanticsProperties.Error))
        compose.onNodeWithTag("name").assertHeightIsAtLeast(56.dp).performTextInput("a".repeat(70))
        compose.runOnIdle { assertEquals(64, name.length) }
        compose.onNodeWithTag("fieldCounter").assertTextEquals("64/64")
        compose.onNodeWithTag("name").assert(SemanticsMatcher.keyNotDefined(androidx.compose.ui.semantics.SemanticsProperties.Error))

        // Slider with an icon in the track (Expressive M): 52 dp handle, TalkBack still sets it.
        compose.onNodeWithTag("volume").assertHeightIsAtLeast(52.dp)
            .performSemanticsAction(SemanticsActions.SetProgress) { it(.75f) }
        compose.runOnIdle { assertTrue("level $level", kotlin.math.abs(level - .75f) < 1e-4) }

        compose.onNodeWithTag("area").assertIsNotEnabled()
    }
}
