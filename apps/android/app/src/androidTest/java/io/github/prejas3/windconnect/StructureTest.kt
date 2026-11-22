package io.github.prejas3.windconnect

import android.content.res.Configuration
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import io.github.prejas3.windconnect.ui.ThemeMode
import io.github.prejas3.windconnect.ui.WindButton
import io.github.prejas3.windconnect.ui.WindConnectTheme
import io.github.prejas3.windconnect.ui.WindDialog
import io.github.prejas3.windconnect.ui.WindIconButton
import io.github.prejas3.windconnect.ui.WindMenuButton
import io.github.prejas3.windconnect.ui.WindMenuItem
import io.github.prejas3.windconnect.ui.WindRadioRow
import io.github.prejas3.windconnect.ui.WindScreen
import io.github.prejas3.windconnect.ui.WindSection
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

/** WN-170: the shared screen structure: top bar, focus ring, tooltip, menu, radio, dialog, sentence-case sections, wide Settings. */
class StructureTest {
    @get:Rule val compose = createComposeRule()

    private fun show(content: @Composable () -> Unit) = compose.setContent {
        WindConnectTheme(mode = ThemeMode.Light, dynamicColor = false) { content() }
    }

    @Test fun screenHasThePinnedBarAndSentenceCaseSections() {
        var back = 0
        show {
            WindScreen("Settings", { back++ }, "Wstecz", "screenBack") {
                WindSection("Phone on the computer") { Text("wiersz") }
            }
        }
        compose.onNodeWithTag("topBar").assertHeightIsEqualTo(64.dp)
        compose.onNodeWithText("Settings").assert(SemanticsMatcher.keyIsDefined(androidx.compose.ui.semantics.SemanticsProperties.Heading))
        compose.onNodeWithText("Phone on the computer").assertExists()
        compose.onNodeWithTag("screenBack").performClick()
        compose.runOnIdle { assertEquals(1, back) }
    }

    @Test fun focusShowsTheSecondaryRing() {
        var secondary = 0
        var input: androidx.compose.ui.input.InputModeManager? = null
        show {
            input = androidx.compose.ui.platform.LocalInputModeManager.current
            secondary = MaterialTheme.colorScheme.secondary.toArgb()
            Column(Modifier.padding(24.dp).testTag("area")) { WindButton("Send", {}, Modifier.testTag("button")) }
        }
        fun ringPixels(): Int {
            val image = compose.onNodeWithTag("area").captureToImage().asAndroidBitmap()
            var count = 0
            for (x in 0 until image.width) for (y in 0 until image.height) if (image.getPixel(x, y) == secondary) count++
            return count
        }
        assertEquals("no ring before focus", 0, ringPixels())
        // Touch never focuses a button (no ring); a keyboard does.
        compose.runOnIdle { input!!.requestInputMode(androidx.compose.ui.input.InputMode.Keyboard) }
        compose.onNodeWithTag("button").requestFocus()
        compose.waitForIdle()
        assertTrue("a ring when focused", ringPixels() > 50)
    }

    @Test fun longPressShowsTheTooltip() {
        show { WindIconButton("pin", "Pin", {}, Modifier.padding(80.dp).testTag("pin")) }
        compose.onNodeWithTag("pin").performTouchInput { longClick() }
        compose.onNodeWithTag("tooltip").assertTextEquals("Pin")
    }

    @Test fun menuRadioAndDialogWork() {
        var picked by mutableStateOf("")
        var chosen by mutableStateOf("a")
        var dialog by mutableStateOf(true)
        show {
            Column {
                WindMenuButton("More", listOf(WindMenuItem("Rename", "rename") { picked = "rename" }, WindMenuItem("Remove", "remove", danger = true) { picked = "remove" }),
                    Modifier.testTag("menuButton"))
                WindRadioRow("Biurko", chosen == "a", { chosen = "a" }, Modifier.testTag("radioA"))
                WindRadioRow("Laptop", chosen == "b", { chosen = "b" }, Modifier.testTag("radioB"), supporting = "1a2b3c4d")
                if (dialog) WindDialog({ dialog = false }, "Open on PC?", actions = { WindButton("Cancel", { dialog = false }, Modifier.testTag("dialogCancel")) }) {
                    Text("example.com")
                }
            }
        }
        compose.onNodeWithTag("windDialog").assertExists()
        compose.onNodeWithText("Open on PC?").assertExists()
        compose.onNodeWithTag("dialogCancel").performClick()
        compose.onNodeWithTag("windDialog").assertDoesNotExist()

        compose.onNodeWithContentDescription("More").performClick()
        compose.onNodeWithTag("remove").performClick()
        compose.runOnIdle { assertEquals("remove", picked) }
        compose.onNodeWithTag("menu").assertDoesNotExist()

        compose.onNodeWithTag("radioA").assertIsSelected()
        compose.onNodeWithTag("radioB").assertIsNotSelected().assertHeightIsAtLeast(48.dp).performClick()
        compose.onNodeWithTag("radioB").assertIsSelected()
        compose.onNodeWithTag("radioA").assertIsNotSelected()
    }

    @Test fun wideSettingsAreAListAndAPanel() {
        show {
            val wide = Configuration(LocalConfiguration.current).apply { screenWidthDp = 900 }
            CompositionLocalProvider(LocalConfiguration provides wide) {
                SettingsPage(listOf("Computer" to { Text("computer row") }, "Shortcuts" to { Text("shortcuts row") }), {})
            }
        }
        compose.onNodeWithTag("settingsList").assertExists()
        compose.onNodeWithText("computer row").assertExists()
        compose.onNodeWithText("shortcuts row").assertDoesNotExist()
        compose.onNodeWithTag("settingsGroup1").performClick()
        compose.onNodeWithText("shortcuts row").assertExists()
        compose.onNodeWithTag("settingsGroup1").assertIsSelected()
    }

    @Test fun narrowSettingsAreOneList() {
        show {
            val narrow = Configuration(LocalConfiguration.current).apply { screenWidthDp = 400 }
            CompositionLocalProvider(LocalConfiguration provides narrow) {
                SettingsPage(listOf("Computer" to { Text("computer row") }, "Shortcuts" to { Text("shortcuts row") }), {})
            }
        }
        compose.onNodeWithTag("settingsList").assertDoesNotExist()
        compose.onNodeWithText("computer row").assertExists()
        compose.onNodeWithText("shortcuts row").assertExists()
    }
}
