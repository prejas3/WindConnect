package io.github.prejas3.windconnect

import android.content.ClipboardManager
import android.content.Context
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import io.github.prejas3.windconnect.bindings.RemoteAction
import io.github.prejas3.windconnect.ui.ButtonKind
import io.github.prejas3.windconnect.ui.WindButton
import io.github.prejas3.windconnect.ui.WindCard
import io.github.prejas3.windconnect.ui.WindField
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

/** WN-100: what the person reads for each status of a PC action (PROTOCOL.md §9.2). */
object PcActionText {
    fun of(status: String, label: String): String = when (status) {
        "done" -> str(R.string.pc_action_done, label)
        "refused" -> str(R.string.pc_action_refused, label)
        "bad-argument" -> str(R.string.pc_action_bad_url, label)
        "busy" -> str(R.string.pc_action_busy, label)
        "unavailable" -> str(R.string.pc_action_unavailable)
        else -> str(R.string.pc_action_failed, label)
    }

    /** The first http(s) link in shared or copied text, or null. */
    fun link(text: String?): String? = text?.let { Regex("""https?://[^\s<>"]+""", RegexOption.IGNORE_CASE).find(it)?.value?.trimEnd('.', ',', ')', ';') }
}

class PcActionsState(private val context: Context, private val scope: CoroutineScope) {
    val status: MutableState<String?> = mutableStateOf(null)
    val asking: MutableState<Boolean> = mutableStateOf(false)
    val typed: MutableState<String> = mutableStateOf("")
    val custom: MutableState<List<RemoteAction>> = mutableStateOf(emptyList())
    private val connections get() = (context.applicationContext as WindApplication).connections

    fun run(id: String, label: String, url: String? = null) {
        scope.launch { status.value = PcActionText.of(connections.runPcAction(id, url), label) }
    }

    val lock: () -> Unit = { run("lock", str(R.string.action_lock_pc)) }

    /** The last link: the clipboard's (read while the app is in front), else a box to type or paste one. */
    val openLink: () -> Unit = {
        val clip = context.getSystemService(ClipboardManager::class.java)?.primaryClip
        val text = if (clip != null && clip.itemCount > 0) clip.getItemAt(0).coerceToText(context)?.toString() else null
        val link = PcActionText.link(text)
        if (link != null) run("open-url", str(R.string.action_open_on_pc), link) else { asking.value = true; status.value = null }
    }

    fun refresh() {
        scope.launch { custom.value = connections.pcActions().filter { it.id.startsWith("x-") } }
    }
}

@Composable
fun rememberPcActions(): PcActionsState {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    return remember { PcActionsState(context, scope) }
}

@Composable
fun PcActionsCard(state: PcActionsState, connected: Boolean) {
    val status by state.status
    val asking by state.asking
    var typed by state.typed
    val custom by state.custom
    androidx.compose.runtime.LaunchedEffect(connected) { if (connected) state.refresh() else state.custom.value = emptyList() }
    if (!connected || (status == null && !asking && custom.isEmpty())) return
    WindCard {
        Text(str(R.string.pc_actions_title), style = MaterialTheme.typography.titleMedium)
        if (asking) {
            Text(str(R.string.pc_link_prompt), style = MaterialTheme.typography.bodySmall)
            WindField(typed, { typed = it }, "https://…", Modifier.fillMaxWidth().testTag("pcLinkField"))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                WindButton(str(R.string.action_open_on_pc), {
                    state.asking.value = false
                    state.run("open-url", str(R.string.action_open_on_pc), typed.trim())
                    typed = ""
                }, Modifier.testTag("pcLinkSend"))
                WindButton(str(R.string.cancel), { state.asking.value = false }, kind = ButtonKind.Quiet)
            }
        }
        for (action in custom) {
            WindButton(action.label, { state.run(action.id, action.label) }, Modifier.fillMaxWidth().testTag("pcAction:${action.id}"), kind = ButtonKind.Secondary)
        }
        status?.let {
            Text(it, style = MaterialTheme.typography.bodySmall, modifier = Modifier.testTag("pcActionStatus").semantics { liveRegion = LiveRegionMode.Polite })
        }
    }
}

