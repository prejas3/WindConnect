package io.github.prejas3.windconnect

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
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
        if (url != null && url.isBlank()) "Podaj adres przekaźnika" else { set(url); null }
    } catch (error: Exception) {
        val message = error.message?.takeIf { it.isNotBlank() }
        // CCS/1 RULE:only a refused URL is "Nieprawidłowy adres"; a core still starting says just that (WN-139).
        if (error is io.github.prejas3.windconnect.bindings.CoreException.InvalidInput || error is IllegalArgumentException)
            "Nieprawidłowy adres: " + (message ?: "użyj wss:// (ws:// tylko w sieci lokalnej)")
        else message ?: "Nie udało się zmienić ustawienia przekaźnika"
    }
}

/** Settings: "Przekaźnik poza domem". The switch turns the relay on with the typed address, or off at once. */
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
    WindCard {
        Text("Przekaźnik poza domem", style = MaterialTheme.typography.titleMedium)
        WindSwitch("Łącz przez przekaźnik, gdy komputer jest poza siecią", wanted, { on ->
            wanted = on
            error = RelaySetting.apply(set, if (on) draft.trim() else null)
            if (error != null) wanted = relay != null
        }, Modifier.testTag("relayEnabled"))
        OutlinedTextField(draft, { draft = it; error = null }, Modifier.fillMaxWidth().testTag("relayUrl"),
            label = { Text("Adres przekaźnika") }, placeholder = { Text(RelaySetting.HINT) }, singleLine = true,
            isError = error != null, supportingText = error?.let { { Text(it, Modifier.testTag("relayError")) } },
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri))
        if (relay != null && draft.trim() != relay) WindButton("Zapisz adres", {
            error = RelaySetting.apply(set, draft.trim())
        }, Modifier.testTag("relaySave"), kind = ButtonKind.Secondary)
        Text("Gdy telefon i komputer nie są w tej samej sieci, połączenie może iść przez przekaźnik. " +
            "Treść jest szyfrowana od końca do końca. Operator przekaźnika widzi adresy IP obu urządzeń, ich klucze publiczne, " +
            "czas połączeń oraz liczbę i rozmiar przesyłanych danych, ale nie ich treść ani nazwy urządzeń. " +
            "Wyłączenie natychmiast kończy połączenie przez przekaźnik.",
            style = MaterialTheme.typography.bodySmall)
    }
}
