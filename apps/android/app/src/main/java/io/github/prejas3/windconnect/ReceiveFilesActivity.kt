package io.github.prejas3.windconnect

import android.app.DownloadManager
import android.content.Context
import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.prejas3.windconnect.core.ReceivedFile
import io.github.prejas3.windconnect.ui.*
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

/** WN-130: every received file (the home card shows the newest few); also where a several-files notice leads. */
class ReceiveFilesActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // WN-170: a file no app could open is said here, on the list (no Toast).
        val problem = intent?.getStringExtra(PROBLEM)
        setContent { WindApp(forceContent = true) {
            WindScreen(str(R.string.received_files), { finish() }, str(R.string.back), "closeReceived") {
                if (!problem.isNullOrEmpty()) WindNotice(problem, Modifier.testTag("receivedProblem"))
                ReceiveFilesControls(compact = false)
            }
        } }
    }
    companion object {
        /** A message to show over the list (a received file no app can open). */
        const val PROBLEM = "problem"
    }
}

/** How many received files the home card lists before "Show all". */
internal const val RECEIVED_ON_HOME = 3

private fun cleanName(name: String) = name.filterNot { it.isISOControl() || it in '‪'..'‮' || it in '⁦'..'⁩' || it == '‎' || it == '‏' }.take(240)

/**
 * WN-130: received files as a compact list. Running transfers (progress, cancel) first, then the files: a tap opens one.
 * [compact] (home): the newest [RECEIVED_ON_HOME] and "Show all (N)". Full (its own screen): all, "Open Downloads" and
 * "Clear list". Turning receiving on or off is in Settings ([ReceiveSettingRow]).
 */
@Composable
internal fun ReceiveFilesControls(state: ReceiveUi = FileReceiving.ui.collectAsState().value, compact: Boolean = true) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var openError by remember { mutableStateOf("") }
    val colors = MaterialTheme.colorScheme
    val active = state.transfers.filter { it.state !in listOf("done", "failed", "cancelled") }
    WindCard(Modifier.fillMaxWidth().testTag("receiveFilesControls")) {
        if (compact) Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(str(R.string.received_files), style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f).semantics { heading() })
            if (state.receipts.size > RECEIVED_ON_HOME) WindButton(str(R.string.received_show_all, state.receipts.size),
                { context.startActivity(Intent(context, ReceiveFilesActivity::class.java)) }, Modifier.testTag("receivedShowAll"), kind = ButtonKind.Quiet)
        }
        Text(if (!state.enabled) str(R.string.received_off) else if (state.ready) str(R.string.receive_ready) else str(R.string.receive_waiting_ready),
            style = MaterialTheme.typography.bodySmall, color = colors.onSurfaceVariant)
        if (state.error.isNotEmpty()) Text(state.error, color = colors.error, style = MaterialTheme.typography.bodySmall)
        active.forEach { file ->
            Text(cleanName(file.name), style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
            val fraction = if (file.size == 0uL) 0f else (file.done.toDouble() / file.size.toDouble()).coerceIn(0.0, 1.0).toFloat()
            // WN-169: the shared progress (wavy while a file moves).
            WindLinearProgress(fraction, Modifier.semantics { contentDescription = str(R.string.receive_progress) }, wavy = true, thick = true)
            Text(if (file.state == "waiting") str(R.string.receive_waiting_connection) else if (file.done == file.size) str(R.string.receive_publishing)
                else str(R.string.bytes_of, transferBytes(file.done.toLong()), transferBytes(file.size.toLong())), style = MaterialTheme.typography.bodySmall, color = colors.onSurfaceVariant)
        }
        if (active.isNotEmpty()) WindButton(str(R.string.receive_cancel), { FileReceiving.cancelAll() }, kind = ButtonKind.Secondary)
        val shown = if (compact) state.receipts.take(RECEIVED_ON_HOME) else state.receipts
        if (shown.isEmpty() && active.isEmpty() && state.enabled) Text(str(R.string.received_none), style = MaterialTheme.typography.bodyMedium, modifier = Modifier.testTag("receivedEmpty"))
        Column {
            shown.forEachIndexed { index, receipt ->
                if (index > 0) WindDivider()
                ReceivedRow(receipt) { open(context, scope, receipt) { openError = it } }
            }
        }
        if (!compact) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                WindButton(str(R.string.open_downloads), {
                    try { context.startActivity(Intent(DownloadManager.ACTION_VIEW_DOWNLOADS)) }
                    catch (_: android.content.ActivityNotFoundException) { openError = str(R.string.receive_no_downloads_app) }
                }, Modifier.testTag("openDownloads"), kind = ButtonKind.Secondary)
                if (state.receipts.isNotEmpty()) WindButton(str(R.string.clear_list), { FileReceiving.clearHistory() }, Modifier.testTag("clearReceived"), kind = ButtonKind.Quiet)
            }
            if (state.receipts.isNotEmpty()) Text(str(R.string.received_tap_opens), style = MaterialTheme.typography.bodySmall, color = colors.onSurfaceVariant)
        }
        if (openError.isNotEmpty()) Text(openError, color = colors.error)
    }
}

/** One received file: type badge, name, size and folder; the whole row opens it. */
@Composable
private fun ReceivedRow(receipt: ReceivedFile, onOpen: () -> Unit) {
    val colors = MaterialTheme.colorScheme
    val name = cleanName(receipt.name)
    val openLabel = str(R.string.open_named, name)
    Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).clip(WindShapes.medium).clickable(role = Role.Button, onClickLabel = openLabel, onClick = onOpen)
        .testTag("received-${receipt.key}").padding(vertical = 8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        val ext = name.substringAfterLast('.', "").uppercase().take(4)
        Box(Modifier.size(40.dp).clip(WindShapes.medium).background(colors.surface), contentAlignment = Alignment.Center) {
            if (ext.isNotEmpty()) Text(ext, style = MaterialTheme.typography.labelSmall, color = colors.primary) else WindIcon("file", Modifier.size(20.dp))
        }
        Column(Modifier.weight(1f)) {
            Text(name, style = MaterialTheme.typography.bodyMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text("${transferBytes(receipt.size)} · ${receipt.folder}", style = MaterialTheme.typography.bodySmall, color = colors.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

private fun open(context: Context, scope: CoroutineScope, receipt: ReceivedFile, fail: (String) -> Unit) {
    scope.launch {
        val uri = FileReceiving.openUri(receipt.key)
        if (uri == null) fail(str(R.string.receive_file_gone))
        else try {
            val view = Intent(Intent.ACTION_VIEW).setDataAndType(uri, context.contentResolver.getType(uri) ?: "application/octet-stream")
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            context.startActivity(Intent.createChooser(view, str(R.string.receive_open_file)))
        } catch (_: android.content.ActivityNotFoundException) { fail(str(R.string.receive_no_app)) }
    }
}

/** WN-130 Settings: receiving on or off (was a button on the home card). */
@Composable
internal fun ReceiveSettingRow(state: ReceiveUi = FileReceiving.ui.collectAsState().value) {
    val context = LocalContext.current
    WindSettingRow("download", str(R.string.receive_title), str(R.string.sum_receive), checked = state.enabled,
        onCheckedChange = { FileReceiving.setEnabled(context, it) }, switchTag = "receiveEnabled",
        more = { Text(str(R.string.receive_where)); Text(str(R.string.receive_when)) })
}
