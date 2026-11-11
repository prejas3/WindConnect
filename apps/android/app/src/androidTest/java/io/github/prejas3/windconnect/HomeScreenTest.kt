package io.github.prejas3.windconnect

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
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
        for (tag in listOf("homeSend", "homeTouchpad", "homeLock", "homeLink")) compose.onNodeWithTag(tag).assertIsNotEnabled()
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
}
