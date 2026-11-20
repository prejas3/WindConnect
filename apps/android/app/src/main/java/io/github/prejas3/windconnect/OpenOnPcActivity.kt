package io.github.prejas3.windconnect

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.annotation.VisibleForTesting
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import io.github.prejas3.windconnect.ui.WindCard
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
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/**
 * WN-100: str(R.string.action_open_on_pc) in the share sheet. SEC:the activity is exported (any app can start it with an explicit intent), so
 * nothing goes to the PC without the person's tap: it asks "Open on <PC>?" with the address's host and a shortened link,
 * and only str(R.string.action_open_on_pc) sends `open-url` (§9.2). Cancel, back or closing sends nothing; no "don't ask again". Without a link
 * or a connection it says so and does nothing.
 */
class OpenOnPcActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val link = PcActionText.link(intent?.takeIf { it.action == Intent.ACTION_SEND }?.getStringExtra(Intent.EXTRA_TEXT))
        if (link == null) {
            Toast.makeText(this, str(R.string.open_no_link), Toast.LENGTH_SHORT).show()
            finish(); return
        }
        val connections = (application as WindApplication).connections
        val pc = testPeer ?: (connections.state.value as? ConnectionState.Connected)?.peerName
        if (pc == null) {
            Toast.makeText(this, str(R.string.files_no_connection), Toast.LENGTH_SHORT).show()
            finish(); return
        }
        val send: suspend (String) -> String = testSender ?: { url -> connections.runPcAction("open-url", url) }
        val host = Uri.parse(link).host.orEmpty()
        val shown = if (link.length > 80) link.take(77) + "…" else link
        setContent {
            WindConnectTheme {
                val scope = rememberCoroutineScope()
                var sending by remember { mutableStateOf(false) }
                // WN-156: own dialog (WindCard, title, WindButton row) instead of Material's AlertDialog.
                Dialog(onDismissRequest = { if (!sending) finish() }) {
                  WindCard(Modifier.fillMaxWidth()) {
                    Text(str(R.string.open_on_pc_question, pc), style = MaterialTheme.typography.titleLarge, modifier = Modifier.semantics { heading() })
                    Text("$host\n$shown", modifier = Modifier.testTag("openOnPcLink"))
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp, androidx.compose.ui.Alignment.End)) {
                        WindButton(str(R.string.cancel), enabled = !sending, onClick = { finish() }, modifier = Modifier.testTag("openOnPcCancel"), kind = ButtonKind.Quiet)
                        WindButton(str(R.string.action_open_on_pc), enabled = !sending, onClick = {
                            sending = true
                            scope.launch {
                                val status = withTimeoutOrNull(8_000) { send(link) } ?: "failed"
                                Toast.makeText(applicationContext, PcActionText.of(status, str(R.string.action_open_on_pc)), Toast.LENGTH_SHORT).show()
                                finish()
                            }
                        }, modifier = Modifier.testTag("openOnPcConfirm"))
                    }
                  }
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
