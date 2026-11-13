package io.github.prejas3.windconnect

import android.app.DownloadManager
import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.background
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import io.github.prejas3.windconnect.ui.*
import kotlinx.coroutines.launch

class ReceiveFilesActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent { WindApp(forceContent = true) {
            Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                ReceiveFilesControls()
                WindButton("Zamknij", { finish() }, kind = ButtonKind.Secondary)
            }
        } }
    }
}

@Composable
internal fun ReceiveFilesControls(state: ReceiveUi = FileReceiving.ui.collectAsState().value) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var openError by remember { mutableStateOf("") }
    Text("Odbiór plików", style = MaterialTheme.typography.titleMedium)
    WindCard(Modifier.fillMaxWidth().testTag("receiveFilesControls")) {
        Text(if (!state.enabled) "Odbiór wyłączony" else if (state.ready) "Gotowy · Pobrane/WindConnect" else "Oczekiwanie na gotowość odbioru")
        Text("Pliki trafiają do publicznych Pobranych po sprawdzeniu sumy kontrolnej. Limit: 2 GiB na plik, 4 GiB aktywnych transferów.", style = MaterialTheme.typography.bodySmall)
        Text("Odbiór działa przy otwartej aplikacji lub po włączeniu „Połączenie w tle”. Bez zgody na powiadomienia postęp i anulowanie znajdziesz tutaj.", style = MaterialTheme.typography.bodySmall)
        WindButton(if (state.enabled) "Wyłącz odbiór" else "Włącz odbiór", { FileReceiving.setEnabled(context, !state.enabled) }, kind = ButtonKind.Secondary)
        if (state.error.isNotEmpty()) Text(state.error, color = MaterialTheme.colorScheme.error)
        state.transfers.filter { it.state !in listOf("done", "failed", "cancelled") }.forEach { file ->
            Text(file.name.filterNot { it.isISOControl() || it in '\u202a'..'\u202e' || it in '\u2066'..'\u2069' || it == '\u200e' || it == '\u200f' }.take(240))
            val fraction = if (file.size == 0uL) 0f else (file.done.toDouble() / file.size.toDouble()).coerceIn(0.0, 1.0).toFloat()
            Box(Modifier.fillMaxWidth().height(8.dp).background(MaterialTheme.colorScheme.surfaceVariant).semantics {
                progressBarRangeInfo = ProgressBarRangeInfo(fraction, 0f..1f)
                contentDescription = "Postęp odbioru"
            }) { Box(Modifier.fillMaxWidth(fraction).fillMaxHeight().background(MaterialTheme.colorScheme.primary)) }
            Text(if (file.state == "waiting") "Oczekiwanie na połączenie" else if (file.done == file.size) "Publikowanie w Pobranych…" else "${transferBytes(file.done.toLong())} z ${transferBytes(file.size.toLong())}")
        }
        if (state.transfers.any { it.state !in listOf("done", "failed", "cancelled") }) WindButton("Anuluj odbiór", { FileReceiving.cancelAll() }, kind = ButtonKind.Secondary)
        WindButton("Pokaż w Pobranych", {
            try { context.startActivity(Intent(DownloadManager.ACTION_VIEW_DOWNLOADS)) }
            catch (_: android.content.ActivityNotFoundException) { openError = "Brak aplikacji do przeglądania Pobranych" }
        }, kind = ButtonKind.Secondary)
        state.receipts.take(10).forEach { receipt ->
            Text(receipt.name, style = MaterialTheme.typography.titleSmall)
            Text(receipt.folder, style = MaterialTheme.typography.bodySmall)
            WindButton("Otwórz ${receipt.name}", {
                scope.launch {
                    val uri = FileReceiving.openUri(receipt.key)
                    if (uri == null) openError = "Plik został przeniesiony lub usunięty"
                    else try {
                        val view = Intent(Intent.ACTION_VIEW).setDataAndType(uri, context.contentResolver.getType(uri) ?: "application/octet-stream")
                            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                        context.startActivity(Intent.createChooser(view, "Otwórz plik"))
                    } catch (_: android.content.ActivityNotFoundException) { openError = "Brak aplikacji do otwarcia tego pliku" }
                }
            }, kind = ButtonKind.Secondary)
        }
        if (state.receipts.isNotEmpty()) {
            Text("Wyczyszczenie historii zachowa pliki w Pobranych. Ponowne wysłanie starego transferu może wtedy utworzyć kolejną kopię.", style = MaterialTheme.typography.bodySmall)
            WindButton("Wyczyść historię odbioru", { FileReceiving.clearHistory() }, kind = ButtonKind.Quiet)
        }
        if (openError.isNotEmpty()) Text(openError, color = MaterialTheme.colorScheme.error)
    }
}
