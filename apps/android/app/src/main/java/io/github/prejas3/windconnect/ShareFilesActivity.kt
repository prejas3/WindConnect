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
                error = str(R.string.share_peers_failed)
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
                error = str(R.string.share_read_failed)
            }
        }
    }
    fun fail(message: String) { error = message }
    fun prepareRetry(ready: (List<Uri>) -> Unit) {
        val selection = FileRetry.selection ?: return fail(str(R.string.share_pick_again))
        previewReady = false
        retryTarget = selection.target; selected = selection.target
        viewModelScope.launch {
            try {
                val opened = core ?: error(str(R.string.share_not_ready))
                check(opened.pairedDevices().any { it.fingerprint == selection.target }) { str(R.string.share_not_paired) }
                withContext(Dispatchers.IO) {
                    FileImport.checkedUris(selection.uris).forEach { uri ->
                        application.contentResolver.openFileDescriptor(uri, "r").use { check(it != null) }
                    }
                }
                ready(selection.uris); preview(selection.uris)
            } catch (failure: Exception) {
                if (failure is CancellationException) throw failure
                error = str(R.string.share_retry_failed)
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
        catch (_: Exception) { model.fail(str(R.string.share_bad_selection)) }
    }
    @Suppress("DEPRECATION")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        try {
            selectedUris = savedInstanceState?.getParcelableArrayList<Uri>("selected")
                ?: if (intent.action == PICK) emptyList() else FileImport.uris(intent)
            model.preview(selectedUris)
        } catch (_: Exception) { model.fail(str(R.string.share_pick_how)) }
        setContent {
            val sending by FileSending.status.collectAsStateWithLifecycle()
            WindApp(forceContent = true) {
                Column(Modifier.fillMaxSize().padding(24.dp).testTag("shareFiles"),
                    verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    Text(str(R.string.share_title), style = MaterialTheme.typography.headlineSmall, modifier = Modifier.semantics { heading() })
                    if (sending.message.isNotEmpty()) WindCard(Modifier.fillMaxWidth()) {
                        Text(if (sending.busy) str(R.string.share_sending) else str(R.string.share_last), style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
                        Text(sending.message, Modifier.testTag("fileSendStatus"))
                        if (!sending.busy && sending.canRetry) WindButton(str(R.string.share_retry), {
                            model.prepareRetry { selectedUris = it }
                        }, Modifier.testTag("retryFiles"), kind = ButtonKind.Secondary)
                        if (sending.busy) WindButton(str(R.string.share_cancel), { SendFilesService.cancel(this@ShareFilesActivity) }, Modifier.testTag("cancelFiles"), kind = ButtonKind.Secondary)
                    }
                    Column(Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    if (sending.busy) {
                        if (sending.target.isNotEmpty()) Text(str(R.string.share_to, sending.target))
                        Text(str(R.string.share_counts, sending.count, sending.completed))
                        TransferProgress(sending)
                        sending.rows.forEach { row ->
                            Text(row.name, style = MaterialTheme.typography.titleSmall)
                            val rowState = str(when (row.state) { "done" -> R.string.row_done; "waiting" -> R.string.row_waiting; "failed" -> R.string.row_failed; "cancelled" -> R.string.row_cancelled; else -> R.string.row_progress })
                            Text(str(R.string.bytes_of, transferBytes(row.done), transferBytes(row.size)) + " · " + rowState, style = MaterialTheme.typography.bodySmall)
                        }
                        Text(str(R.string.share_close_ok), style = MaterialTheme.typography.bodySmall)
                    } else {
                    WindButton(str(R.string.share_pick), { pendingPicker = true; openPendingPicker() }, Modifier.testTag("pickFiles"), enabled = !sending.busy, kind = ButtonKind.Secondary)
                    Text(str(R.string.share_selected, selectedUris.size), Modifier.testTag("selectedFileCount"))
                    Text(str(R.string.share_target), style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
                    if (model.peers.isEmpty()) Text(str(R.string.share_pair_first))
                    model.peers.forEach { peer ->
                        WindButton("${computerLabel(peer.name)} · ${peer.fingerprint.take(8)}", { model.selected = peer.fingerprint },
                            Modifier.fillMaxWidth().semantics { selected = model.selected == peer.fingerprint }, enabled = !sending.busy && (model.retryTarget == null || model.retryTarget == peer.fingerprint),
                            kind = if (model.selected == peer.fingerprint) ButtonKind.Primary else ButtonKind.Secondary)
                    }
                    if (model.error.isNotEmpty()) Text(model.error, color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("fileShareError"))
                    WindButton(str(R.string.send), {
                        try { SendFilesService.start(this@ShareFilesActivity, selectedUris, requireNotNull(model.selected)) }
                        catch (_: Exception) { model.fail(str(R.string.share_start_failed)) }
                    }, Modifier.fillMaxWidth().testTag("confirmSendFiles"), enabled = !sending.busy && model.previewReady && model.peers.any { it.fingerprint == model.selected })
                    Text(str(R.string.share_where), style = MaterialTheme.typography.bodySmall)
                    model.names.forEach { Text(it, Modifier.fillMaxWidth(), style = MaterialTheme.typography.bodySmall) }
                    }
                    WindButton(str(R.string.close), { finish() }, kind = ButtonKind.Quiet)
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
    Box(Modifier.fillMaxWidth().height(8.dp).background(colors.surfaceContainerHigh).testTag("transferProgress").semantics {
        progressBarRangeInfo = fraction?.let { ProgressBarRangeInfo(it, 0f..1f) } ?: ProgressBarRangeInfo.Indeterminate
        contentDescription = str(R.string.share_progress)
    }) {
        Box(Modifier.fillMaxWidth(fraction ?: .35f).fillMaxHeight().background(colors.primary))
    }
    val bytes = total?.let { str(R.string.bytes_of, transferBytes(status.done), transferBytes(it)) } ?: (transferBytes(status.done) + " · " + str(R.string.size_unknown))
    Text(bytes + " · " + (status.rate?.let { str(R.string.per_second, transferBytes(it)) } ?: "—"), Modifier.testTag("transferBytes"))
}
