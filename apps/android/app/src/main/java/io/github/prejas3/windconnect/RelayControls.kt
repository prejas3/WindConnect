package io.github.prejas3.windconnect

import io.github.prejas3.windconnect.ui.*
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.heading

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.foundation.text.KeyboardOptions
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.prejas3.windconnect.ui.ButtonKind
import io.github.prejas3.windconnect.ui.WindButton
import io.github.prejas3.windconnect.ui.WindCard
import io.github.prejas3.windconnect.ui.WindSwitch

/** CCS/1 M:relay setting (WN-92) | RULE:off by default (WN-60 D3); a refused URL stays in the field with the error. */
internal object RelaySetting {
    /** Example shown in the field; the real address comes from relay/README.md after deployment. */
    const val HINT = "wss://windconnect-relay.<subdomena>.workers.dev"

    /** Applies [url] (null: off); returns the error to show at the field, or null when it was taken. */
    fun apply(set: (String?) -> Unit, url: String?): String? = try {
        if (url != null && url.isBlank()) str(R.string.relay_address_needed) else { set(url); null }
    } catch (error: Exception) {
        val message = error.message?.takeIf { it.isNotBlank() }
        // CCS/1 RULE:only a refused URL is "Nieprawidłowy adres"; a core still starting says just that (WN-139).
        if (error is io.github.prejas3.windconnect.bindings.CoreException.InvalidInput || error is IllegalArgumentException)
            str(R.string.relay_invalid, message ?: str(R.string.relay_use_wss))
        else message ?: str(R.string.relay_change_failed)
    }
}

/** Settings: str(R.string.relay_title). The switch turns the relay on with the typed address, or off at once. */
@Composable
fun RelayControls() {
    val connections = (LocalContext.current.applicationContext as WindApplication).connections
    val relay by connections.relayUrl.collectAsStateWithLifecycle()
    RelayCard(relay, connections::setRelay)
}

@Composable
internal fun RelayCard(relay: String?, set: (String?) -> Unit) {
    var draft by remember(relay) { mutableStateOf(relay ?: "") }
    var error by remember { mutableStateOf<String?>(null) }
    var wanted by remember(relay) { mutableStateOf(relay != null) }
    WindSettingRow("relay", str(R.string.relay_title), str(R.string.sum_relay), checked = wanted, switchTag = "relayEnabled",
        onCheckedChange = { on ->
            wanted = on
            error = RelaySetting.apply(set, if (on) draft.trim() else null)
            if (error != null) wanted = relay != null
        },
        actions = {
            WindField(draft, { draft = it; error = null }, str(R.string.relay_address) + " (" + RelaySetting.HINT + ")", Modifier.testTag("relayUrl"),
                keyboardType = KeyboardType.Uri)
            error?.let { Text(it, Modifier.testTag("relayError"), color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
            if (relay != null && draft.trim() != relay) WindButton(str(R.string.relay_save), {
                error = RelaySetting.apply(set, draft.trim())
            }, Modifier.testTag("relaySave"), kind = ButtonKind.Secondary)
        },
        more = { Text(str(R.string.relay_about)) })
}
