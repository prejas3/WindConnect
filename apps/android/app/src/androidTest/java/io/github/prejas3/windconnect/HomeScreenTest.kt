package io.github.prejas3.windconnect

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.ui.WindAction
import io.github.prejas3.windconnect.ui.WindCard
import org.junit.Assert.assertEquals
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import io.github.prejas3.windconnect.core.ConnectionState
import io.github.prejas3.windconnect.ui.ThemeMode
import io.github.prejas3.windconnect.ui.WindConnectTheme
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

class HomeScreenTest {
    @get:Rule val compose = createComposeRule()

    @Test fun unpairedHomeOffersRealPairingAndNoInventedTransfers() {
        var scan = false
        compose.setContent { WindConnectTheme(ThemeMode.Light, false) {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                HomeScreen(PairingUi(ready = true, initialized = true), { scan = true }, onDiagnostics = {})
            }
        } }
        compose.onNodeWithTag("computerName").assertTextEquals("Twój komputer")
        compose.onNodeWithTag("computerLatency").assertTextEquals("Brak sparowanego komputera")
        for (tag in listOf("homeSend", "homeTouchpad", "homeLock", "homeLink"))
            compose.onNodeWithTag(tag).assertIsNotEnabled().assertTextContains("Niedostępne")
        compose.onNodeWithText("Nie ma jeszcze transferów").assertExists()
        compose.onNodeWithTag("scanQr").performScrollTo().performClick()
        assertTrue(scan)
    }

    @Test fun disconnectKeepsComputerNameButRemovesStaleLatency() {
        val state = mutableStateOf(PairingUi(ready = true, initialized = true, hasPairedDevice = true,
            computerName = "Mój PC", connection = ConnectionState.Connected("Mój PC", "test", 1234)))
        compose.setContent { WindConnectTheme(ThemeMode.Dark, false) {
            Column(Modifier.verticalScroll(rememberScrollState())) { HomeScreen(state.value, {}, onDiagnostics = {}) }
        } }
        compose.onNodeWithTag("computerName").assertTextEquals("Mój PC")
        compose.onNodeWithTag("computerLatency").assertTextContains("ms", substring = true)
        compose.runOnIdle { state.value = state.value.copy(connection = ConnectionState.Disconnected("test")) }
        compose.onNodeWithTag("computerName").assertTextEquals("Mój PC")
        compose.onNodeWithTag("computerLatency").assertTextEquals("Komputer poza zasięgiem")
    }

    @Test fun unavailableActionAtLargeFontDoesNotRunAndBecomesNormalWhenAvailable() {
        val mode = mutableStateOf(ThemeMode.Light)
        val available = mutableStateOf(false)
        var calls = 0
        compose.setContent {
            val density = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(density.density, 1.5f)) {
                WindConnectTheme(mode.value, false) {
                    WindCard(Modifier.fillMaxWidth().testTag("actionComparison")) {
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            WindAction("Wyślij plik", "Do komputera", "send", { calls++ }, Modifier.weight(1f).testTag("availableAction"))
                            WindAction("Touchpad", "i klawiatura", "touch", if (available.value) ({ calls++ }) else null,
                                Modifier.weight(1f).testTag("unavailableAction"))
                        }
                    }
                }
            }
        }
        for (theme in listOf(ThemeMode.Light, ThemeMode.Dark)) {
            compose.runOnIdle { mode.value = theme }
            compose.onNodeWithTag("unavailableAction").assertIsNotEnabled().assertTextContains("Niedostępne")
                .performTouchInput { click() }
            compose.runOnIdle { assertEquals(0, calls) }
            compose.onNodeWithTag("availableAction").assertIsEnabled()
            val image = compose.onNodeWithTag("actionComparison").captureToImage().asAndroidBitmap()
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            java.io.File(context.cacheDir, "wn102-${theme.name}-150.png").outputStream().use {
                image.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it)
            }
            compose.onNodeWithText("Niedostępne", useUnmergedTree = true).assertIsDisplayed()
                .performSemanticsAction(androidx.compose.ui.semantics.SemanticsActions.GetTextLayoutResult) { action ->
                    val layouts = mutableListOf<androidx.compose.ui.text.TextLayoutResult>()
                    action(layouts)
                    assertTrue("disabled label must fit at150%: ${layouts.map { "${it.size}/${it.lineCount}/width=${it.didOverflowWidth}/height=${it.didOverflowHeight}" }}", layouts.none { it.hasVisualOverflow })
                }
        }
        compose.runOnIdle { available.value = true }
        compose.onNodeWithTag("unavailableAction").assertIsEnabled().performClick()
        compose.onNodeWithText("Niedostępne", useUnmergedTree = true).assertDoesNotExist()
        compose.runOnIdle { assertEquals(1, calls) }
    }
}
