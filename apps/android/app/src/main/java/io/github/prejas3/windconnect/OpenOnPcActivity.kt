package io.github.prejas3.windconnect

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.annotation.VisibleForTesting
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import io.github.prejas3.windconnect.ui.ButtonKind
import io.github.prejas3.windconnect.ui.WindButton
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import io.github.prejas3.windconnect.core.ConnectionState
import io.github.prejas3.windconnect.ui.WindConnectTheme
import io.github.prejas3.windconnect.ui.WindDialog
import io.github.prejas3.windconnect.ui.WindLinearProgress
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/**
 * WN-100: str(R.string.action_open_on_pc) in the share sheet. WN-170: an own WindDialog; the outcome is shown in it. SEC:the activity is exported (any app can start it with an explicit intent), so
 * nothing goes to the PC without the person's tap: it asks "Open on <PC>?" with the address's host and a shortened link,
 * and only str(R.string.action_open_on_pc) sends `open-url` (§9.2). Cancel, back or closing sends nothing; no "don't ask again". Without a link
 * or a connection it says so and does nothing.
 */
class OpenOnPcActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val link = PcActionText.link(intent?.takeIf { it.action == Intent.ACTION_SEND }?.getStringExtra(Intent.EXTRA_TEXT))
        val connections = (application as WindApplication).connections
        val pc = testPeer ?: (connections.state.value as? ConnectionState.Connected)?.peerName
        // WN-170: what can't be done is said in the same dialog (no Toast): no link, or no computer connected.
        val problem = when { link == null -> str(R.string.open_no_link); pc == null -> str(R.string.files_no_connection); else -> null }
        val send: suspend (String) -> String = testSender ?: { url -> connections.runPcAction("open-url", url) }
        val host = link?.let { Uri.parse(it).host.orEmpty() }.orEmpty()
        val shown = link?.let { if (it.length > 80) it.take(77) + "…" else it }.orEmpty()
        setContent {
            WindConnectTheme {
                val scope = rememberCoroutineScope()
                var sending by remember { mutableStateOf(false) }
                // The outcome: the PC's answer, shown in the dialog with a check (or a warning) until closed.
                var outcome by remember { mutableStateOf(problem?.let { false to it }) }
                val close = { finish() }
                val result = outcome
                if (result != null) {
                    val (done, text) = result
                    WindDialog(close, text, Modifier.testTag("openOnPcResult"), icon = if (done) "check" else "warning",
                        iconTint = if (done) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.error,
                        actions = { WindButton(str(R.string.close), close, Modifier.testTag("openOnPcDone"), kind = ButtonKind.Quiet) }) {}
                } else WindDialog({ if (!sending) finish() }, str(R.string.open_on_pc_question, pc.orEmpty()), actions = {
                    WindButton(str(R.string.cancel), enabled = !sending, onClick = close, modifier = Modifier.testTag("openOnPcCancel"), kind = ButtonKind.Quiet)
                    WindButton(str(R.string.action_open_on_pc), enabled = !sending, onClick = {
                        sending = true
                        scope.launch {
                            val status = withTimeoutOrNull(8_000) { send(link!!) } ?: "failed"
                            outcome = (status == "done") to PcActionText.of(status, str(R.string.action_open_on_pc))
                            sending = false
                        }
                    }, modifier = Modifier.testTag("openOnPcConfirm"))
                }) {
                    Text("$host\n$shown", modifier = Modifier.testTag("openOnPcLink"))
                    if (sending) WindLinearProgress(null, Modifier.testTag("openOnPcSending"))
                }
            }
        }
    }

    companion object {
        /** Tests only: what str(R.string.action_open_on_pc) sends to, and the PC's name (no live session needed). */
        @VisibleForTesting @JvmStatic var testSender: (suspend (String) -> String)? = null
        @VisibleForTesting @JvmStatic var testPeer: String? = null
    }
}
