package io.github.prejas3.windconnect

import androidx.compose.ui.res.stringResource
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
    Text(stringResource(R.string.devices_paired), style = MaterialTheme.typography.headlineSmall)
    WindButton(stringResource(R.string.back), onBack, Modifier.testTag("closeDevices"), kind = ButtonKind.Quiet)
    if (state.message.isNotEmpty()) Text(state.message, modifier = Modifier.testTag("devicesMessage").semantics { liveRegion = LiveRegionMode.Polite }, style = MaterialTheme.typography.bodyMedium)
    val connected = state.connection as? ConnectionState.Connected
    var editing by rememberSaveable { mutableStateOf<String?>(null) }
    var asking by rememberSaveable { mutableStateOf<String?>(null) }
    for (entry in state.devices) {
        val isConnected = connected?.fingerprint == entry.fingerprint
        val label = computerLabel(entry.name)
        WindCard(Modifier.fillMaxWidth().testTag("device-${entry.fingerprint.take(8)}")) {
            Text(if (entry.owed) stringResource(R.string.device_removed) else label, style = MaterialTheme.typography.titleMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
            val date = DateFormat.getDateInstance(DateFormat.MEDIUM).format(Date(entry.pairedAt.toLong() * 1000))
            Text(
                when {
                    entry.owed -> stringResource(R.string.device_removed_at, date)
                    isConnected -> stringResource(R.string.device_connected_at, date)
                    else -> stringResource(R.string.device_disconnected_at, date)
                },
                style = MaterialTheme.typography.bodySmall,
                color = if (isConnected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(stringResource(R.string.fingerprint_short, shortFingerprint(entry.fingerprint)), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (entry.owed) return@WindCard
            when {
                editing == entry.fingerprint -> {
                    var name by remember(entry.fingerprint) { mutableStateOf(label) }
                    var empty by remember { mutableStateOf(false) }
                    WindField(name, { name = it.take(64); empty = false }, stringResource(R.string.device_rename_label), Modifier.testTag("renameField"), errorText = if (empty) stringResource(R.string.device_name_empty) else null)
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        WindButton(stringResource(R.string.save), { if (name.isBlank()) empty = true else { editing = null; onRename(entry, name.trim()) } }, Modifier.testTag("renameSave"))
                        WindButton(stringResource(R.string.cancel), { editing = null }, kind = ButtonKind.Quiet)
                    }
                }
                asking == entry.fingerprint -> {
                    Text(
                        stringResource(if (isConnected) R.string.unpair_question_now else R.string.unpair_question_later, label),
                        style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier.testTag("unpairQuestion"),
                    )
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        WindButton(stringResource(R.string.unpair), { asking = null; onUnpair(entry) }, Modifier.testTag("unpairConfirm"))
                        WindButton(stringResource(R.string.cancel), { asking = null }, kind = ButtonKind.Quiet)
                    }
                }
                else -> FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (!isConnected) WindButton(stringResource(R.string.connect), { onConnect(entry) }, Modifier.testTag("deviceConnect"), kind = ButtonKind.Secondary)
                    WindButton(stringResource(R.string.rename), { editing = entry.fingerprint; asking = null }, Modifier.testTag("deviceRename"), kind = ButtonKind.Quiet)
                    WindButton(stringResource(R.string.remove), { asking = entry.fingerprint; editing = null }, Modifier.testTag("deviceUnpair"), kind = ButtonKind.Quiet)
                }
            }
        }
    }
    if (state.devices.isEmpty()) Text(stringResource(R.string.devices_none), style = MaterialTheme.typography.bodyMedium)
}
