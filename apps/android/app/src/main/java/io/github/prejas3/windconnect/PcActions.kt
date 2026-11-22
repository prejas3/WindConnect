package io.github.prejas3.windconnect

import androidx.compose.ui.semantics.heading

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
import kotlinx.coroutines.withTimeoutOrNull

/** WN-100: what the person reads for each status of a PC action (PROTOCOL.md §9.2). */
object PcActionText {
    fun of(status: String, label: String, id: String? = null): String = when {
        // WN-173: a link the PC took but its browser didn't open says so (not a bare "failed").
        status == "failed" && id == "open-url" -> str(R.string.pc_link_failed)
        else -> of(status, label)
    }

    private fun of(status: String, label: String): String = when (status) {
        "done" -> str(R.string.pc_action_done, label)
        "refused" -> str(R.string.pc_action_refused, label)
        "bad-argument" -> str(R.string.pc_action_bad_url, label)
        "busy" -> str(R.string.pc_action_busy, label)
        "unavailable" -> str(R.string.pc_action_unavailable)
        else -> str(R.string.pc_action_failed, label)
    }

    /**
     * WN-165: a PC action's name in the phone's language, by its id (PROTOCOL.md §9.2): the PC's label is only a
     * fallback, for a custom action's own name and for a built-in this version doesn't know.
     */
    fun label(id: String, sent: String): String = when (id) {
        "lock" -> str(R.string.action_lock_pc)
        "sleep" -> str(R.string.action_sleep_pc)
        "open-url" -> str(R.string.action_open_on_pc)
        else -> sent.trim().ifEmpty { id }
    }

    /** The first http(s) link in shared or copied text, or null. */
    fun link(text: String?): String? = text?.let { Regex("""https?://[^\s<>"]+""", RegexOption.IGNORE_CASE).find(it)?.value?.trimEnd('.', ',', ')', ';') }

    /**
     * WN-173: [link] as the PC takes it (wind-core `check_url`: printable ASCII only, no `"<>\^`{|}`, an ASCII host).
     * CCS/1 M:WN-173 cause (one of two): a link copied as text keeps its letters (https://pl.wikipedia.org/wiki/Łódź,
     * an IDN host, a `|` in a query), the PC answered bad-argument and nothing opened. Here the host goes to punycode and
     * every other byte the PC refuses is percent-encoded as UTF-8; an existing %XX stays as it is.
     */
    fun forPc(link: String): String {
        val schemeEnd = link.indexOf("://").takeIf { it > 0 } ?: return link
        val afterScheme = schemeEnd + 3
        val authorityEnd = link.indexOfAny(charArrayOf('/', '?', '#'), afterScheme).let { if (it < 0) link.length else it }
        val authority = link.substring(afterScheme, authorityEnd)
        val (host, port) = if (authority.startsWith("[")) authority to "" else authority.substringBeforeLast(':', authority).let { h ->
            h to authority.removePrefix(h)
        }
        val asciiHost = if (host.all { it.code in 0x21..0x7e }) host else try {
            java.net.IDN.toASCII(host, java.net.IDN.ALLOW_UNASSIGNED)
        } catch (_: IllegalArgumentException) { host }
        return link.substring(0, afterScheme).lowercase() + encode(asciiHost) + encode(port) + encode(link.substring(authorityEnd))
    }

    private fun encode(part: String): String = buildString {
        for (byte in part.toByteArray(Charsets.UTF_8)) {
            val b = byte.toInt() and 0xff
            if (b in 0x21..0x7e && b.toChar() !in "\"<>\\^`{|}") append(b.toChar()) else append('%').append("%02X".format(b))
        }
    }
}

class PcActionsState(private val context: Context, private val scope: CoroutineScope) {
    val status: MutableState<String?> = mutableStateOf(null)
    val asking: MutableState<Boolean> = mutableStateOf(false)
    val typed: MutableState<String> = mutableStateOf("")
    val custom: MutableState<List<RemoteAction>> = mutableStateOf(emptyList())
    private val connections get() = (context.applicationContext as WindApplication).connections

    /**
     * WN-173: the status shows at once ("sending…") and under the home tiles (pcActionStatus), so a tap is never silent;
     * a PC that doesn't answer in 8 s counts as failed, as in the share sheet.
     */
    fun run(id: String, label: String, url: String? = null) {
        status.value = str(R.string.pc_action_sending, label)
        scope.launch {
            val sent = url?.let(PcActionText::forPc)
            val answer = withTimeoutOrNull(8_000) { connections.runPcAction(id, sent) } ?: "failed"
            status.value = PcActionText.of(answer, label, id)
        }
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
        // Lock and Open on PC have their own buttons on the home screen; the rest (sleep, custom, newer) go here.
        scope.launch { custom.value = connections.pcActions().filter { it.id !in OWN_BUTTONS } }
    }
}

private val OWN_BUTTONS = setOf("lock", "open-url")

/** The PC's actions listed for the phone, each named in the phone's language where its id is known (WN-165). */
@Composable
fun PcActionButtons(actions: List<RemoteAction>, onRun: (id: String, label: String) -> Unit) {
    for (action in actions) {
        val label = PcActionText.label(action.id, action.label)
        WindButton(label, { onRun(action.id, label) }, Modifier.fillMaxWidth().testTag("pcAction:${action.id}"), kind = ButtonKind.Tonal)
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
    val asking by state.asking
    var typed by state.typed
    val custom by state.custom
    androidx.compose.runtime.LaunchedEffect(connected) { if (connected) state.refresh() else state.custom.value = emptyList() }
    if (!connected || (!asking && custom.isEmpty())) return
    WindCard {
        Text(str(R.string.pc_actions_title), style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
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
        PcActionButtons(custom) { id, label -> state.run(id, label) }
    }
}

/** WN-173: the last PC action's outcome, shown under the home tiles that started it (not in a card further down). */
@Composable
fun PcActionStatus(state: PcActionsState) {
    val status by state.status
    status?.let {
        Text(it, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.testTag("pcActionStatus").semantics { liveRegion = LiveRegionMode.Polite })
    }
}

