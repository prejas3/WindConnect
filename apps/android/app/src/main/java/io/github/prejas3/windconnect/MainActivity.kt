package io.github.prejas3.windconnect

import androidx.compose.ui.res.stringResource
import android.os.Bundle
import android.view.KeyEvent
import androidx.compose.foundation.layout.Box
import androidx.compose.ui.Alignment
import android.content.Intent
import androidx.activity.viewModels
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.compose.ReportDrawnWhen
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
    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        (application as WindApplication).connections.clipboardFocus(hasFocus)
    }
    private val pairing: PairingViewModel by viewModels()
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        receiveLink(intent)
        setContent {
            val state by pairing.ui.collectAsStateWithLifecycle()
            // CCS/1 PERF:TTFD includes local identity/store readiness; an offline PC cannot hold startup reporting indefinitely.
            ReportDrawnWhen { state.initialized }
            val connections = (application as WindApplication).connections
            val ringing by connections.ringing.collectAsStateWithLifecycle()
            Box(Modifier.fillMaxSize()) {
                WindApp(forceContent = state.screen != "home", onSettings = pairing::settings) { PairingScreen(pairing) { ConnectionScreen() } }
                // WN-50: a way to stop that doesn't need notifications (POST_NOTIFICATIONS may be refused).
                if (ringing) StopRingingBar(connections::stopRinging, Modifier.align(Alignment.TopCenter))
            }
        }
    }
    // WN-50: a volume key stops the ring while the app is open (no extra permission needed).
    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
        val connections = (application as WindApplication).connections
        if ((keyCode == KeyEvent.KEYCODE_VOLUME_DOWN || keyCode == KeyEvent.KEYCODE_VOLUME_UP) && connections.ringing.value) {
            connections.stopRinging()
            return true
        }
        return super.onKeyDown(keyCode, event)
    }
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        receiveLink(intent)
    }
    override fun onStart() {
        super.onStart()
        ConnectionService.start(this)
    }
    private fun receiveLink(incoming: Intent) {
        // CCS/1 SEC:never retain the QR secret in Activity intent/saved state; every VIEW is validated in Rust.
        if (incoming.action == Intent.ACTION_VIEW) {
            val link = incoming.dataString
            incoming.data = null
            // CCS/1 RULE:configuration recreation must not re-offer an already consumed VIEW intent.
            if (link != null) pairing.offerLink(link)
        }
    }
}

data class ConnectionUiState(
    val publicKey: String = "",
    val fingerprint: String = "",
    val status: String = str(R.string.diag_starting),
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
                    val opened = (application as WindApplication).connections.acquire()
                    core = opened
                    mutableUi.value = ConnectionUiState(
                        publicKey = opened.identity.publicKey.joinToString("") { "%02x".format(it) },
                        fingerprint = opened.identity.displayFingerprint,
                        status = str(R.string.diag_ready), ready = true,
                    )
                    owner.launch {
                        opened.state.collect { state ->
                            mutableUi.update { current ->
                                current.copy(
                                    busy = state is ConnectionState.Connecting,
                                    status = when (state) {
                                        ConnectionState.Ready -> str(R.string.diag_ready)
                                        ConnectionState.Connecting -> str(R.string.connecting)
                                        is ConnectionState.Connected -> str(R.string.diag_connected, state.peerName, (state.roundTripMicros / 1000.0).toString())
                                        is ConnectionState.Disconnected -> str(R.string.diag_disconnected, state.reason)
                                        is ConnectionState.Failed -> str(R.string.error_prefixed, endMessage(state) ?: state.message)
                                        ConnectionState.Stopped -> str(R.string.diag_stopped)
                                    },
                                )
                            }
                        }
                    }
                } catch (error: Exception) {
                    if (error is CancellationException) throw error
                    mutableUi.update { it.copy(status = str(R.string.error_prefixed, error.message.orEmpty())) }
                }
            }
        }
    }

    fun connect(address: String, keyHex: String) {
        owner.launch {
            gate.withLock {
                try {
                    require(keyHex.length == 64 && keyHex.all { it in "0123456789abcdefABCDEF" }) {
                        str(R.string.diag_key_length)
                    }
                    val key = ByteArray(32) { keyHex.substring(it * 2, it * 2 + 2).toInt(16).toByte() }
                    core?.connect(address.trim(), key)
                } catch (error: Exception) {
                    if (error is CancellationException) throw error
                    mutableUi.update { it.copy(status = str(R.string.error_prefixed, error.message.orEmpty()), busy = false) }
                }
            }
        }
    }

    fun disconnect() { owner.launch { gate.withLock { core?.disconnect() } } }

    override fun onCleared() {
        owner.launch {
            gate.withLock {
                try { core?.let { (getApplication<Application>() as WindApplication).connections.release(it) } } finally { owner.cancel() }
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
        Text(stringResource(R.string.home_diagnostics), style = MaterialTheme.typography.titleLarge)
        Text(stringResource(R.string.diag_fingerprint))
        SelectionContainer { Text(ui.fingerprint) }
        Text(stringResource(R.string.diag_public_key))
        SelectionContainer { Text(ui.publicKey, modifier = Modifier.testTag("localKey")) }
        WindField(
            value = address, onValueChange = { address = it }, label = stringResource(R.string.diag_address),
            modifier = Modifier.fillMaxWidth().testTag("peerAddress"), singleLine = true,
        )
        WindField(
            value = key, onValueChange = { key = it.trim() }, label = stringResource(R.string.diag_key_label),
            modifier = Modifier.fillMaxWidth().testTag("peerKey"),
        )
        Text(stringResource(R.string.diag_compare))
        WindButton(stringResource(R.string.diag_connect), onClick = { model.connect(address, key) }, enabled = ui.ready && !ui.busy,
            modifier = Modifier.testTag("connect"))
        WindButton(stringResource(R.string.disconnect), onClick = model::disconnect, enabled = ui.ready && !ui.busy, kind = ButtonKind.Secondary)
        Text(ui.status, modifier = Modifier.testTag("connectionStatus"))
    }
}
