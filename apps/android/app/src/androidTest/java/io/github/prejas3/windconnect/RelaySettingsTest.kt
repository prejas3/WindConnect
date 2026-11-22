package io.github.prejas3.windconnect

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.bindings.CoreException
import io.github.prejas3.windconnect.core.ConnectionState
import io.github.prejas3.windconnect.core.RELAY_OFF
import io.github.prejas3.windconnect.core.WindCore
import io.github.prejas3.windconnect.ui.ThemeMode
import io.github.prejas3.windconnect.ui.WindConnectTheme
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test

/** WN-92: the relay switch (off by default), its saved address, the route on screen, and off ending a relay session at once. */
class RelaySettingsTest {
    @get:Rule val compose = createComposeRule()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    @Test fun switchIsOffByDefaultShowsRefusedUrlAndTurnsOnAndOff() {
        val relay = mutableStateOf<String?>(null)
        val calls = mutableListOf<String?>()
        compose.setContent { WindConnectTheme(ThemeMode.Light, false) {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                RelayCard(relay.value) { url ->
                    calls += url
                    if (url != null && !url.startsWith("wss://")) throw IllegalArgumentException("must start with wss://")
                    relay.value = url
                }
            }
        } }
        compose.onNodeWithTag("relayEnabled").assertIsOff()
        compose.onNodeWithTag("relayUrl").performTextInput("http://relay.example")
        compose.onNodeWithTag("relayEnabled").performClick()
        compose.onNodeWithTag("relayError", useUnmergedTree = true).assertTextContains("must start with wss://", substring = true)
        compose.onNodeWithTag("relayEnabled").assertIsOff()
        compose.onNodeWithTag("relayUrl").performTextClearance()
        compose.onNodeWithTag("relayUrl").performTextInput("wss://windconnect-relay.example.workers.dev")
        compose.onNodeWithTag("relayError", useUnmergedTree = true).assertDoesNotExist()
        compose.onNodeWithTag("relayEnabled").performClick()
        compose.onNodeWithTag("relayEnabled").assertIsOn()
        compose.onNodeWithTag("relayEnabled").performClick()
        compose.onNodeWithTag("relayEnabled").assertIsOff()
        assertEquals(listOf("http://relay.example", "wss://windconnect-relay.example.workers.dev", null), calls)
    }

    @Test fun homeShowsTheRelayRoute() {
        val state = mutableStateOf(PairingUi(ready = true, initialized = true, hasPairedDevice = true, computerName = "PC",
            connection = ConnectionState.Connected("PC", "f", 9000, route = "relay")))
        compose.setContent { WindConnectTheme(ThemeMode.Light, false) {
            Column(Modifier.verticalScroll(rememberScrollState())) { HomeScreen(state.value, {}, onDiagnostics = {}) }
        } }
        compose.onNodeWithTag("pairingStatus").assertTextEquals("Connected through the relay")
        compose.onNodeWithTag("computerLatency").assertTextContains("via relay (E2E)", substring = true)
        compose.runOnIdle { state.value = state.value.copy(connection = ConnectionState.Connected("PC", "f", 900)) }
        compose.onNodeWithTag("pairingStatus").assertTextEquals("Connected")
        compose.onNodeWithTag("computerLatency").assertTextContains("local network", substring = true)
    }

    @Test fun addressSurvivesARestartAndARefusedOneIsNotSaved() = runBlocking {
        val context = instrumentation.targetContext
        var core = WindCore.open(context, "Android")
        try {
            core.setRelay("wss://windconnect-relay.example.workers.dev")
            assertThrows(CoreException.InvalidInput::class.java) { core.setRelay("http://relay.example") }
            core.shutdown()
            core = WindCore.open(context, "Android")
            assertEquals("wss://windconnect-relay.example.workers.dev", core.relayUrl.value)
            core.setRelay(null)
            core.shutdown()
            core = WindCore.open(context, "Android")
            assertNull(core.relayUrl.value)
        } finally { core.setRelay(null); core.shutdown() }
    }

    /** Reports this phone's key for `windctl serve --trust` (instrumentation status "phoneKey"). */
    @Test fun reportPhoneKey() = runBlocking {
        val core = WindCore.open(instrumentation.targetContext, "Android")
        try {
            instrumentation.sendStatus(0, android.os.Bundle().apply {
                putString("phoneKey", core.identity.publicKey.joinToString("") { "%02x".format(it) })
            })
        } finally { core.shutdown() }
    }

    /** With `windctl serve --relay` and a local relay (relay/README.md): off ends the relay session at once. */
    @Test fun turningTheRelayOffEndsARelaySessionAtOnce() = runBlocking {
        val args = InstrumentationRegistry.getArguments()
        val key = args.getString("pcKey"); val relay = args.getString("relayUrl"); val away = args.getString("awayAddress")
        assumeTrue("requires pcKey, relayUrl and awayAddress", key != null && relay != null && away != null)
        val core = WindCore.open(instrumentation.targetContext, "Android")
        try {
            core.setRelay(relay)
            core.connect(away!!, key!!.chunked(2).map { it.toInt(16).toByte() }.toByteArray())
            val connected = withTimeout(20_000) { core.state.first { it is ConnectionState.Connected } as ConnectionState.Connected }
            assertEquals("relay", connected.route)
            val off = System.nanoTime()
            core.setRelay(null)
            val ended = withTimeout(2_000) { core.state.first { it !is ConnectionState.Connected } }
            val tookMs = (System.nanoTime() - off) / 1_000_000
            assertTrue("ended: $ended", ended is ConnectionState.Disconnected && ended.reason == RELAY_OFF)
            // The LAN try that follows cannot reach the away address, and nothing goes back to the relay.
            val after = withTimeout(30_000) { core.state.first { it !is ConnectionState.Connecting && it !is ConnectionState.Disconnected } }
            assertFalse("no relay session after off: $after", after is ConnectionState.Connected)
            instrumentation.sendStatus(0, android.os.Bundle().apply { putString("offMs", tookMs.toString()); putString("after", after.toString()) })
        } finally { core.setRelay(null); core.shutdown() }
    }

    /** WN-139: off during the 2 s LAN wait of a connect that already read the relay; the session it lands on leaves the relay. */
    @Test fun turningTheRelayOffDuringAConnectLeavesNoRelaySession() = runBlocking {
        val args = InstrumentationRegistry.getArguments()
        val key = args.getString("pcKey"); val relay = args.getString("relayUrl"); val away = args.getString("awayAddress")
        assumeTrue("requires pcKey, relayUrl and awayAddress", key != null && relay != null && away != null)
        val core = WindCore.open(instrumentation.targetContext, "Android")
        val seen = java.util.Collections.synchronizedList(mutableListOf<ConnectionState>())
        val record = launch(Dispatchers.Default) { core.state.collect { seen += it } }
        try {
            core.setRelay(relay)
            val connect = launch(Dispatchers.IO) {
                try { core.connect(away!!, key!!.chunked(2).map { it.toInt(16).toByte() }.toByteArray()) }
                catch (error: Exception) { if (error is CancellationException) throw error }
            }
            withTimeout(5_000) { core.state.first { it is ConnectionState.Connecting } }
            delay(800) // Connecting comes just before wind-ffi reads the relay; 0.8 s is well inside its 2 s LAN try
            core.setRelay(null) // wind-ffi has read the relay: this connect still ends on it
            withTimeout(30_000) { connect.join() }
            delay(3_000) // leaveRelay runs once adopt releases the gate
            val now = core.state.value
            assertFalse("still on the relay: $now / $seen", now is ConnectionState.Connected && now.route == "relay")
            // StateFlow conflates: Disconnected(relay-off) is often replaced by the LAN Connecting before it is seen.
            val history = synchronized(seen) { seen.toList() }
            val onRelay = history.indexOfFirst { it is ConnectionState.Connected && it.route == "relay" }
            val landed = onRelay >= 0
            assertTrue("the connect must reach the relay for this race (timing): $history", landed)
            assertTrue("the relay session was left: $history", history.drop(onRelay + 1).isNotEmpty())
            instrumentation.sendStatus(0, android.os.Bundle().apply { putString("landedOnRelay", landed.toString()); putString("after", now.toString()) })
        } finally { record.cancel(); core.setRelay(null); core.shutdown() }
    }
}
