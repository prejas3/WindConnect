package io.github.prejas3.windconnect

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.prejas3.windconnect.core.ConnectionState
import io.github.prejas3.windconnect.ui.*
import java.util.Locale
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.compose.runtime.getValue

// CCS/1 SEC:peer labels are presentation only; strip control/direction overrides without changing trusted keys.
internal fun computerLabel(value: String?): String = value.orEmpty().filterNot {
    it.isISOControl() || it in '\u202a'..'\u202e' || it in '\u2066'..'\u2069' || it == '\u200e' || it == '\u200f'
}.trim().take(64).ifEmpty { "Komputer" }

@Composable
fun HomeScreen(state: PairingUi, onScan: () -> Unit, onDiagnostics: () -> Unit, onNearby: (() -> Unit)? = null, onClipboard: (() -> Unit)? = null, onClipboardEnabled: ((Boolean) -> Unit)? = null, onSendFile: (() -> Unit)? = null, history: ClipboardHistoryActions? = null) {
    val pcActions = rememberPcActions()
    val context = LocalContext.current
    val touchpadAccepted by (context.applicationContext as WindApplication).connections.touchpadAccepted.collectAsStateWithLifecycle()
    val connected = state.connection as? ConnectionState.Connected
    val known = state.hasPairedDevice || connected != null
    val name = if (known) computerLabel(state.computerName) else "Twój komputer"
    val status = when {
        !state.initialized -> "Przygotowywanie połączenia…"
        !state.ready -> "Nie udało się uruchomić połączenia"
        connected != null -> if (connected.route == "relay") "Połączono przez przekaźnik" else "Połączono"
        state.connection is ConnectionState.Connecting -> "Łączenie…"
        known -> "Komputer poza zasięgiem"
        else -> "Brak sparowanego komputera"
    }
    val colors = MaterialTheme.colorScheme
    // WC-34: first run, the Welcome card comes first (what is needed, one main action, the camera explained).
    if (!known) WelcomeCard(state.ready, onScan, onNearby)
    Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(22.dp)).background(colors.primaryContainer).padding(18.dp).testTag("connectionStrip"),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        WindIcon("pc")
        Column(Modifier.weight(1f)) {
            Text(name, style = MaterialTheme.typography.labelLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(status, style = MaterialTheme.typography.bodySmall, color = colors.onPrimaryContainer, modifier = Modifier.testTag("pairingStatus"))
        }
    }
    WindCard(Modifier.fillMaxWidth().testTag("computerCard")) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Box(Modifier.size(44.dp).clip(RoundedCornerShape(13.dp)).background(colors.surface), contentAlignment = Alignment.Center) { WindIcon("pc") }
            Text(name, modifier = Modifier.weight(1f).testTag("computerName"), style = MaterialTheme.typography.titleLarge, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(7.dp)) {
            Box(Modifier.size(6.dp).clip(RoundedCornerShape(3.dp)).background(if (connected != null) colors.primary else colors.onSurfaceVariant))
            val latency = connected?.let { String.format(Locale.getDefault(), "%.1f ms", it.roundTripMicros / 1000.0) }
            Text(if (connected != null) "$latency · ${if (connected.route == "relay") "przez przekaźnik (E2E)" else "sieć lokalna"}" else status,
                style = MaterialTheme.typography.bodySmall, color = colors.onSurfaceVariant,
                modifier = Modifier.testTag("computerLatency").semantics { contentDescription = if (latency != null) "Ostatni pomiar opóźnienia: $latency" else status })
        }
        // CCS/1 RULE:actions are disabled until their real channel integrations exist; no simulated send/lock/link result.
        Row(Modifier.fillMaxWidth().height(IntrinsicSize.Min), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            WindAction("Wyślij plik", "lub udostępnij z galerii", "send", onSendFile?.takeIf { connected != null && state.filesAccepted }, Modifier.weight(1f).fillMaxHeight().testTag("homeSend"))
            WindAction("Touchpad", "kursor i gesty", "touch", if (connected != null && touchpadAccepted) { { context.startActivity(android.content.Intent(context, TouchpadActivity::class.java)) } } else null, Modifier.weight(1f).fillMaxHeight().testTag("homeTouchpad"))
        }
        Row(Modifier.fillMaxWidth().height(IntrinsicSize.Min), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            // WN-100: the PC's actions (PcActions.kt).
            WindAction("Zablokuj PC", "Win+L zdalnie", "lock", pcActions.lock.takeIf { connected != null }, Modifier.weight(1f).fillMaxHeight().testTag("homeLock"))
            WindAction("Otwórz na PC", "ostatni link", "external", pcActions.openLink.takeIf { connected != null }, Modifier.weight(1f).fillMaxHeight().testTag("homeLink"))
        }
        Text("Akcje oznaczone jako niedostępne nie są jeszcze gotowe lub komputer ich teraz nie przyjmuje.", color = colors.onSurfaceVariant, style = MaterialTheme.typography.bodySmall)
    }
    if (onClipboard != null && onClipboardEnabled != null) ClipboardControls(state, onClipboard, onClipboardEnabled)
    if (history != null) ClipboardHistoryCard(state.clipboardHistory, state.clipboard.enabled, history)
    PcMediaCard() // WN-47: the PC's player; shown only when the PC has one.
    PcVolumeCard() // WN-48: the PC's volume; shown while the PC shares it.
    PcActionsCard(pcActions, connected != null) // WN-100: the outcome, the link box and the PC's own actions.
    if (known) ReceiveFilesControls()
    Text("Ostatnie transfery", style = MaterialTheme.typography.titleMedium)
    WindCard(Modifier.fillMaxWidth().testTag("recentTransfers")) {
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            WindIcon("file")
            Column(Modifier.weight(1f)) {
                Text("Nie ma jeszcze transferów", style = MaterialTheme.typography.labelLarge)
                Text("Wysłane i odebrane pliki znajdziesz w tym miejscu.", color = colors.onSurfaceVariant, style = MaterialTheme.typography.bodySmall)
            }
        }
    }
    if (!state.ready && state.message.isNotEmpty()) Text(state.message, color = colors.error)
    if (state.skippedRecords > 0uL) Text("Pominięto uszkodzone wpisy sparowania: ${state.skippedRecords}. Poprawne urządzenia są nadal dostępne.", modifier = Modifier.testTag("pairingStorageWarning"), style = MaterialTheme.typography.bodySmall)
    if (BuildConfig.DEBUG) WindButton("Test połączenia z PC", onDiagnostics, kind = ButtonKind.Quiet)
}

@Composable
@OptIn(ExperimentalMaterial3Api::class)
fun ClipboardControls(state: PairingUi, onSend: () -> Unit, onEnabled: (Boolean) -> Unit, onImageLimit: ((Int) -> Unit)? = null) {
    WindCard(Modifier.fillMaxWidth().testTag("clipboardCard")) {
        Text("Wspólny schowek", style = MaterialTheme.typography.titleMedium)
        WindSwitch("Synchronizacja automatyczna", state.clipboard.enabled, onEnabled, Modifier.testTag("clipboardEnabled"))
        Text(state.clipboard.status, style = MaterialTheme.typography.bodySmall, modifier = Modifier.testTag("clipboardStatus"))
        Text("Tekst, linki i obrazy. PC → telefon także w tle. Telefon → PC po otwarciu aplikacji lub z akcji powiadomienia. Treści oznaczone jako wrażliwe nie są wysyłane.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        if (onImageLimit != null) {
            // WN-36: images over this size are neither sent nor taken.
            Text("Maks. rozmiar obrazu", style = MaterialTheme.typography.bodyMedium)
            val limits = listOf(5, 16, 50)
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().testTag("clipboardImageLimit")) {
                limits.forEachIndexed { index, mb ->
                    SegmentedButton(selected = state.clipboard.imageLimitMb == mb, onClick = { onImageLimit(mb) },
                        shape = SegmentedButtonDefaults.itemShape(index, limits.size), modifier = Modifier.testTag("imageLimit$mb")) { Text("$mb MB") }
                }
            }
        }
        WindButton("Wyślij schowek", onSend, Modifier.fillMaxWidth().testTag("sendClipboard"), enabled = state.connection is ConnectionState.Connected && !state.clipboard.busy, kind = ButtonKind.Secondary)
    }
}
