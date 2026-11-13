package io.github.prejas3.windconnect

import android.app.Application
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.background
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.prejas3.windconnect.bindings.PeerInfo
import io.github.prejas3.windconnect.core.ConnectionState
import io.github.prejas3.windconnect.core.WindCore
import io.github.prejas3.windconnect.ui.*
import kotlinx.coroutines.*

class FileShareViewModel(private val application: Application) : AndroidViewModel(application) {
    var peers by mutableStateOf<List<PeerInfo>>(emptyList()); private set
    var selected by mutableStateOf<String?>(null)
    var error by mutableStateOf(""); private set
    var names by mutableStateOf<List<String>>(emptyList()); private set
    var previewReady by mutableStateOf(false); private set
    var retryTarget by mutableStateOf<String?>(null); private set
    private var core: WindCore? = null
    private var preview: Job? = null
    init {
        viewModelScope.launch {
            try {
                val opened = (application as WindApplication).connections.acquire()
                core = opened
                peers = opened.pairedDevices()
                selected = (opened.state.value as? ConnectionState.Connected)?.fingerprint ?: peers.lastOrNull()?.fingerprint
            } catch (failure: Exception) {
                if (failure is CancellationException) throw failure
                error = "Nie można odczytać sparowanych komputerów. Otwórz aplikację i spróbuj ponownie."
            }
        }
    }
    fun preview(uris: List<Uri>) {
        preview?.cancel(); previewReady = false; names = emptyList(); error = ""
        preview = viewModelScope.launch {
            try {
                val shown = withContext(Dispatchers.IO) {
                    val used = mutableSetOf<String>()
                    uris.map { FileImport.describe(application, it, used).name }
                }
                names = shown; previewReady = shown.isNotEmpty()
            } catch (failure: Exception) {
                if (failure is CancellationException) throw failure
                error = "Nie można odczytać wybranych plików. Wybierz je ponownie."
            }
        }
    }
    fun fail(message: String) { error = message }
    fun prepareRetry(ready: (List<Uri>) -> Unit) {
        val selection = FileRetry.selection ?: return fail("Wybierz pliki ponownie")
        previewReady = false
        retryTarget = selection.target; selected = selection.target
        viewModelScope.launch {
            try {
                val opened = core ?: error("Połączenie nie jest jeszcze gotowe")
                check(opened.pairedDevices().any { it.fingerprint == selection.target }) { "Komputer nie jest już sparowany" }
                withContext(Dispatchers.IO) {
                    FileImport.checkedUris(selection.uris).forEach { uri ->
                        application.contentResolver.openFileDescriptor(uri, "r").use { check(it != null) }
                    }
                }
                ready(selection.uris); preview(selection.uris)
            } catch (failure: Exception) {
                if (failure is CancellationException) throw failure
                error = "Nie można ponowić odczytu. Wybierz ponownie pliki dla tego samego komputera."
                ready(emptyList())
            }
        }
    }
    override fun onCleared() {
        core?.let { opened -> CoroutineScope(Dispatchers.IO).launch { (application as WindApplication).connections.release(opened) } }
        core = null
    }
}

/** CCS/1 SEC:external intents select content only; user selects a trusted target and explicitly sends. */
class ShareFilesActivity : ComponentActivity() {
    private val model by viewModels<FileShareViewModel>()
    private var selectedUris by mutableStateOf<List<Uri>>(emptyList())
    private var pendingPicker = false
    private var pickerInFlight = false
    private val picker = registerForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        pickerInFlight = false
        try { selectedUris = FileImport.checkedUris(uris); model.preview(selectedUris) }
        catch (_: Exception) { model.fail("Nieprawidłowy wybór plików") }
    }
    @Suppress("DEPRECATION")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        try {
            selectedUris = savedInstanceState?.getParcelableArrayList<Uri>("selected")
                ?: if (intent.action == PICK) emptyList() else FileImport.uris(intent)
            model.preview(selectedUris)
        } catch (_: Exception) { model.fail("Wybierz pliki przez udostępnianie lub systemowy wybór plików") }
        setContent {
            val sending by FileSending.status.collectAsStateWithLifecycle()
            WindApp(forceContent = true) {
                Column(Modifier.fillMaxSize().padding(24.dp).testTag("shareFiles"),
                    verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    Text("Wyślij pliki", style = MaterialTheme.typography.headlineSmall)
                    if (sending.message.isNotEmpty()) WindCard(Modifier.fillMaxWidth()) {
                        Text(if (sending.busy) "Trwa wysyłanie" else "Ostatnia wysyłka", style = MaterialTheme.typography.titleMedium)
                        Text(sending.message, Modifier.testTag("fileSendStatus"))
                        if (!sending.busy && sending.canRetry) WindButton("Ponów niewysłane", {
                            model.prepareRetry { selectedUris = it }
                        }, Modifier.testTag("retryFiles"), kind = ButtonKind.Secondary)
                        if (sending.busy) WindButton("Anuluj wysyłanie", { SendFilesService.cancel(this@ShareFilesActivity) }, Modifier.testTag("cancelFiles"), kind = ButtonKind.Secondary)
                    }
                    Column(Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    if (sending.busy) {
                        if (sending.target.isNotEmpty()) Text("Do: ${sending.target}")
                        Text("Pliki: ${sending.count} · odebrano: ${sending.completed}")
                        TransferProgress(sending)
                        sending.rows.forEach { row ->
                            Text(row.name, style = MaterialTheme.typography.titleSmall)
                            Text("${transferBytes(row.done)} z ${transferBytes(row.size)} · ${when(row.state) { "done" -> "Odebrano"; "waiting" -> "Czeka na połączenie"; "failed" -> "Błąd"; "cancelled" -> "Anulowano"; else -> "W toku" }}", style = MaterialTheme.typography.bodySmall)
                        }
                        Text("Możesz zamknąć ekran. Wysyłanie będzie kontynuowane w tle.", style = MaterialTheme.typography.bodySmall)
                    } else {
                    WindButton("Wybierz pliki", { pendingPicker = true; openPendingPicker() }, Modifier.testTag("pickFiles"), enabled = !sending.busy, kind = ButtonKind.Secondary)
                    Text("Wybrane pliki: ${selectedUris.size}", Modifier.testTag("selectedFileCount"))
                    Text("Komputer docelowy", style = MaterialTheme.typography.titleMedium)
                    if (model.peers.isEmpty()) Text("Najpierw sparuj komputer w WindConnect.")
                    model.peers.forEach { peer ->
                        WindButton("${computerLabel(peer.name)} · ${peer.fingerprint.take(8)}", { model.selected = peer.fingerprint },
                            Modifier.fillMaxWidth().semantics { selected = model.selected == peer.fingerprint }, enabled = !sending.busy && (model.retryTarget == null || model.retryTarget == peer.fingerprint),
                            kind = if (model.selected == peer.fingerprint) ButtonKind.Primary else ButtonKind.Secondary)
                    }
                    if (model.error.isNotEmpty()) Text(model.error, color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("fileShareError"))
                    WindButton("Wyślij", {
                        try { SendFilesService.start(this@ShareFilesActivity, selectedUris, requireNotNull(model.selected)) }
                        catch (_: Exception) { model.fail("Nie udało się rozpocząć wysyłki. Spróbuj ponownie przy otwartym ekranie.") }
                    }, Modifier.fillMaxWidth().testTag("confirmSendFiles"), enabled = !sending.busy && model.previewReady && model.peers.any { it.fingerprint == model.selected })
                    Text("Pliki trafią do Pobrane/WindConnect na komputerze, do folderów według typu. Limit: 100 plików, 2 GiB na plik i 4 GiB na wysyłkę.", style = MaterialTheme.typography.bodySmall)
                    model.names.forEach { Text(it, Modifier.fillMaxWidth(), style = MaterialTheme.typography.bodySmall) }
                    }
                    WindButton("Zamknij", { finish() }, kind = ButtonKind.Quiet)
                    }
                }
            }
        }
        pendingPicker = savedInstanceState == null && intent.action == PICK && !FileSending.status.value.busy
        pickerInFlight = savedInstanceState?.getBoolean("pickerInFlight") ?: false
    }
    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus && pendingPicker) {
            val keyguard = getSystemService(android.app.KeyguardManager::class.java)
            if (keyguard.isKeyguardLocked) {
                keyguard.requestDismissKeyguard(this, object : android.app.KeyguardManager.KeyguardDismissCallback() {
                    override fun onDismissSucceeded() { openPendingPicker() }
                })
            } else openPendingPicker()
        }
    }
    private fun openPendingPicker() {
        if (!pendingPicker || pickerInFlight || isFinishing || isDestroyed || !hasWindowFocus()) return
        val keyguard = getSystemService(android.app.KeyguardManager::class.java)
        if (keyguard.isKeyguardLocked) return
        pendingPicker = false
        if (!FileSending.status.value.busy) {
            pickerInFlight = true
            picker.launch(arrayOf("*/*"))
        }
    }
    override fun onSaveInstanceState(outState: Bundle) {
        outState.putBoolean("pickerInFlight", pickerInFlight)
        outState.putParcelableArrayList("selected", ArrayList(selectedUris)); super.onSaveInstanceState(outState)
    }
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        // CCS/1 RULE:a notification tap never replaces selected content or an active batch.
        if (intent.action == PICK && !FileSending.status.value.busy && selectedUris.isEmpty() && !pickerInFlight) {
            pendingPicker = true
            openPendingPicker()
        }
    }
    companion object { const val PICK = "io.github.prejas3.windconnect.PICK_FILES" }
}

@Composable
private fun TransferProgress(status: FileSendStatus) {
    val total = status.total
    val fraction = total?.let { if (it == 0L) 0f else (status.done.toDouble() / it).coerceIn(0.0, 1.0).toFloat() }
    val colors = MaterialTheme.colorScheme
    Box(Modifier.fillMaxWidth().height(8.dp).background(colors.surfaceVariant).testTag("transferProgress").semantics {
        progressBarRangeInfo = fraction?.let { ProgressBarRangeInfo(it, 0f..1f) } ?: ProgressBarRangeInfo.Indeterminate
        contentDescription = "Postęp wysyłania"
    }) {
        Box(Modifier.fillMaxWidth(fraction ?: .35f).fillMaxHeight().background(colors.primary))
    }
    Text("${transferBytes(status.done)}${total?.let { " z ${transferBytes(it)}" } ?: " · rozmiar jeszcze nieznany"} · ${status.rate?.let { transferBytes(it) + "/s" } ?: "—"}", Modifier.testTag("transferBytes"))
}
