package io.github.prejas3.windconnect

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.annotation.VisibleForTesting
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
 * nothing goes to the PC without the person's tap: it asks "Otworzyć na <PC>?" with the address's host and a shortened link,
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
                AlertDialog(
                    onDismissRequest = { if (!sending) finish() },
                    title = { Text(str(R.string.open_on_pc_question, pc)) },
                    text = { Text("$host\n$shown", modifier = Modifier.testTag("openOnPcLink")) },
                    confirmButton = {
                        TextButton(enabled = !sending, onClick = {
                            sending = true
                            scope.launch {
                                val status = withTimeoutOrNull(8_000) { send(link) } ?: "failed"
                                Toast.makeText(applicationContext, PcActionText.of(status, str(R.string.action_open_on_pc)), Toast.LENGTH_SHORT).show()
                                finish()
                            }
                        }, modifier = Modifier.testTag("openOnPcConfirm")) { Text(str(R.string.action_open_on_pc)) }
                    },
                    dismissButton = {
                        TextButton(enabled = !sending, onClick = { finish() }, modifier = Modifier.testTag("openOnPcCancel")) { Text(str(R.string.cancel)) }
                    },
                )
            }
        }
    }

    companion object {
        /** Tests only: what str(R.string.action_open_on_pc) sends to, and the PC's name (no live session needed). */
        @VisibleForTesting @JvmStatic var testSender: (suspend (String) -> String)? = null
        @VisibleForTesting @JvmStatic var testPeer: String? = null
    }
}
