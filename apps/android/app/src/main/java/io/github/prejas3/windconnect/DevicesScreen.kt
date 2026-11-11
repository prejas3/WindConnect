package io.github.prejas3.windconnect

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.prejas3.windconnect.bindings.PairedEntry
import io.github.prejas3.windconnect.core.ConnectionState
import io.github.prejas3.windconnect.ui.ButtonKind
import io.github.prejas3.windconnect.ui.WindButton
import io.github.prejas3.windconnect.ui.WindCard
import io.github.prejas3.windconnect.ui.WindField
import java.text.DateFormat
import java.util.Date

/**
 * WC-28: the paired computers. A local name, connecting by hand (which lifts a BUSY/refusal pause)
 * and removing the pairing on both sides; edits and the removal question stay inside the card.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun DevicesScreen(
    state: PairingUi,
    onBack: () -> Unit,
    onConnect: (PairedEntry) -> Unit,
    onRename: (PairedEntry, String) -> Unit,
    onUnpair: (PairedEntry) -> Unit,
) {
    Text("Sparowane komputery", style = MaterialTheme.typography.headlineSmall)
    WindButton("Powrót", onBack, Modifier.testTag("closeDevices"), kind = ButtonKind.Quiet)
    if (state.message.isNotEmpty()) Text(state.message, modifier = Modifier.testTag("devicesMessage").semantics { liveRegion = LiveRegionMode.Polite }, style = MaterialTheme.typography.bodyMedium)
    val connected = state.connection as? ConnectionState.Connected
    var editing by rememberSaveable { mutableStateOf<String?>(null) }
    var asking by rememberSaveable { mutableStateOf<String?>(null) }
    for (entry in state.devices) {
        val isConnected = connected?.fingerprint == entry.fingerprint
        val label = computerLabel(entry.name)
        WindCard(Modifier.fillMaxWidth().testTag("device-${entry.fingerprint.take(8)}")) {
            Text(if (entry.owed) "Usunięty komputer" else label, style = MaterialTheme.typography.titleMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
            val date = DateFormat.getDateInstance(DateFormat.MEDIUM).format(Date(entry.pairedAt.toLong() * 1000))
            Text(
                when {
                    entry.owed -> "Usunięto $date · komputer dowie się, gdy będzie w tej samej sieci"
                    isConnected -> "Połączony · sparowano $date"
                    else -> "Niepołączony · sparowano $date"
                },
                style = MaterialTheme.typography.bodySmall,
                color = if (isConnected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text("Odcisk: ${shortFingerprint(entry.fingerprint)}", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (entry.owed) return@WindCard
            when {
                editing == entry.fingerprint -> {
                    var name by remember(entry.fingerprint) { mutableStateOf(label) }
                    var empty by remember { mutableStateOf(false) }
                    WindField(name, { name = it.take(64); empty = false }, "Nazwa widoczna na tym telefonie", Modifier.testTag("renameField"), errorText = if (empty) "Nazwa nie może być pusta" else null)
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        WindButton("Zapisz", { if (name.isBlank()) empty = true else { editing = null; onRename(entry, name.trim()) } }, Modifier.testTag("renameSave"))
                        WindButton("Anuluj", { editing = null }, kind = ButtonKind.Quiet)
                    }
                }
                asking == entry.fingerprint -> {
                    Text(
                        "Usunąć parowanie z: $label? Komputer też je usunie${if (isConnected) "" else ", gdy znajdzie się w tej samej sieci"}. Ponowne połączenie wymaga nowego parowania.",
                        style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier.testTag("unpairQuestion"),
                    )
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        WindButton("Usuń parowanie", { asking = null; onUnpair(entry) }, Modifier.testTag("unpairConfirm"))
                        WindButton("Anuluj", { asking = null }, kind = ButtonKind.Quiet)
                    }
                }
                else -> FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (!isConnected) WindButton("Połącz", { onConnect(entry) }, Modifier.testTag("deviceConnect"), kind = ButtonKind.Secondary)
                    WindButton("Zmień nazwę", { editing = entry.fingerprint; asking = null }, Modifier.testTag("deviceRename"), kind = ButtonKind.Quiet)
                    WindButton("Usuń", { asking = entry.fingerprint; editing = null }, Modifier.testTag("deviceUnpair"), kind = ButtonKind.Quiet)
                }
            }
        }
    }
    if (state.devices.isEmpty()) Text("Brak sparowanych komputerów.", style = MaterialTheme.typography.bodyMedium)
}
