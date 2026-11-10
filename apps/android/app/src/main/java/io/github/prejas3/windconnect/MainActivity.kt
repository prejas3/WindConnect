package io.github.prejas3.windconnect

import android.os.Bundle
import android.content.Intent
import androidx.activity.viewModels
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import android.app.Application
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import io.github.prejas3.windconnect.core.ConnectionState
import io.github.prejas3.windconnect.core.WindCore
import io.github.prejas3.windconnect.ui.WindApp
import io.github.prejas3.windconnect.ui.WindButton
import io.github.prejas3.windconnect.ui.WindField
import io.github.prejas3.windconnect.ui.ButtonKind
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

class MainActivity : ComponentActivity() {
    private val pairing: PairingViewModel by viewModels()
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        receiveLink(intent)
        setContent {
            val state by pairing.ui.collectAsStateWithLifecycle()
            WindApp(forceContent = state.screen != "home") { PairingScreen(pairing) { ConnectionScreen() } }
        }
    }
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        receiveLink(intent)
    }
    private fun receiveLink(incoming: Intent) {
        // CCS/1 SEC:never retain the QR secret in Activity intent/saved state; every VIEW is validated in Rust.
        if (incoming.action == Intent.ACTION_VIEW) {
            val link = incoming.dataString
            incoming.data = null
            pairing.offerLink(link)
        }
    }
}

data class ConnectionUiState(
    val publicKey: String = "",
    val fingerprint: String = "",
    val status: String = "Uruchamianie rdzenia…",
    val ready: Boolean = false,
    val busy: Boolean = false,
)

class ConnectionViewModel(application: Application) : AndroidViewModel(application) {
    private val owner = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val gate = Mutex()
    private var core: WindCore? = null
    private val mutableUi = MutableStateFlow(ConnectionUiState())
    val ui = mutableUi.asStateFlow()

    init {
        owner.launch {
            gate.withLock {
                try {
                    val opened = WindCore.open(application, "Android")
                    core = opened
                    mutableUi.value = ConnectionUiState(
                        publicKey = opened.identity.publicKey.joinToString("") { "%02x".format(it) },
                        fingerprint = opened.identity.displayFingerprint,
                        status = "Gotowy", ready = true,
                    )
                    owner.launch {
                        opened.state.collect { state ->
                            mutableUi.update { current ->
                                current.copy(
                                    busy = state is ConnectionState.Connecting,
                                    status = when (state) {
                                        ConnectionState.Ready -> "Gotowy"
                                        ConnectionState.Connecting -> "Łączenie…"
                                        is ConnectionState.Connected -> "Połączono: ${state.peerName}; ping ${state.roundTripMicros / 1000.0} ms"
                                        is ConnectionState.Disconnected -> "Rozłączono: ${state.reason}"
                                        is ConnectionState.Failed -> "Błąd: ${state.message}"
                                        ConnectionState.Stopped -> "Zatrzymano"
                                    },
                                )
                            }
                        }
                    }
                } catch (error: Exception) {
                    if (error is CancellationException) throw error
                    mutableUi.update { it.copy(status = "Błąd: ${error.message}") }
                }
            }
        }
    }

    fun connect(address: String, keyHex: String) {
        owner.launch {
            gate.withLock {
                try {
                    require(keyHex.length == 64 && keyHex.all { it in "0123456789abcdefABCDEF" }) {
                        "Klucz PC musi mieć 64 znaki hex"
                    }
                    val key = ByteArray(32) { keyHex.substring(it * 2, it * 2 + 2).toInt(16).toByte() }
                    core?.connect(address.trim(), key)
                } catch (error: Exception) {
                    if (error is CancellationException) throw error
                    mutableUi.update { it.copy(status = "Błąd: ${error.message}", busy = false) }
                }
            }
        }
    }

    fun disconnect() { owner.launch { gate.withLock { core?.disconnect() } } }

    override fun onCleared() {
        owner.launch {
            gate.withLock {
                try { core?.shutdown() } finally { owner.cancel() }
            }
        }
    }
}

@Composable
private fun ConnectionScreen(model: ConnectionViewModel = viewModel()) {
    val ui by model.ui.collectAsStateWithLifecycle()
    var address by remember { mutableStateOf("") }
    var key by remember { mutableStateOf("") }
    Column(
        modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text("Test połączenia z PC", style = MaterialTheme.typography.titleLarge)
        Text("Odcisk tego urządzenia")
        SelectionContainer { Text(ui.fingerprint) }
        Text("Klucz publiczny Androida — przekaż go do narzędzia testowego na PC")
        SelectionContainer { Text(ui.publicKey, modifier = Modifier.testTag("localKey")) }
        WindField(
            value = address, onValueChange = { address = it }, label = "Adres PC, np. 192.168.1.10:47820",
            modifier = Modifier.fillMaxWidth().testTag("peerAddress"), singleLine = true,
        )
        WindField(
            value = key, onValueChange = { key = it.trim() }, label = "Zweryfikowany klucz publiczny PC (64 hex)",
            modifier = Modifier.fillMaxWidth().testTag("peerKey"),
        )
        Text("Porównaj klucz z tym wyświetlonym na PC. To ręczny test zaufania przed wdrożeniem parowania.")
        WindButton("Połącz z podanym kluczem", onClick = { model.connect(address, key) }, enabled = ui.ready && !ui.busy,
            modifier = Modifier.testTag("connect"))
        WindButton("Rozłącz", onClick = model::disconnect, enabled = ui.ready && !ui.busy, kind = ButtonKind.Secondary)
        Text(ui.status, modifier = Modifier.testTag("connectionStatus"))
    }
}
