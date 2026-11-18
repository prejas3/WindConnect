package io.github.prejas3.windconnect

import androidx.compose.ui.semantics.heading

import androidx.compose.ui.res.stringResource
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
}.trim().take(64).ifEmpty { str(R.string.computer_default) }

@Composable
fun HomeScreen(state: PairingUi, onScan: () -> Unit, onDiagnostics: () -> Unit, onNearby: (() -> Unit)? = null, onClipboard: (() -> Unit)? = null, onClipboardEnabled: ((Boolean) -> Unit)? = null, onSendFile: (() -> Unit)? = null, history: ClipboardHistoryActions? = null) {
    val pcActions = rememberPcActions()
    val context = LocalContext.current
    val touchpadAccepted by (context.applicationContext as WindApplication).connections.touchpadAccepted.collectAsStateWithLifecycle()
    val connected = state.connection as? ConnectionState.Connected
    val known = state.hasPairedDevice || connected != null
    val name = if (known) computerLabel(state.computerName) else stringResource(R.string.your_computer)
    val status = when {
        !state.initialized -> stringResource(R.string.home_preparing)
        !state.ready -> stringResource(R.string.home_start_failed)
        connected != null -> if (connected.route == "relay") stringResource(R.string.home_connected_relay) else stringResource(R.string.connected)
        state.connection is ConnectionState.Connecting -> stringResource(R.string.connecting)
        known -> stringResource(R.string.home_out_of_reach)
        else -> stringResource(R.string.home_no_paired)
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
            val route = stringResource(if (connected?.route == "relay") R.string.home_via_relay else R.string.home_local_network)
            val latencyDescription = if (latency != null) stringResource(R.string.home_latency_desc, latency) else status
            Text(if (connected != null) stringResource(R.string.home_latency_line, latency.orEmpty(), route) else status,
                style = MaterialTheme.typography.bodySmall, color = colors.onSurfaceVariant,
                modifier = Modifier.testTag("computerLatency").semantics { contentDescription = latencyDescription })
        }
        // CCS/1 RULE:actions are disabled until their real channel integrations exist; no simulated send/lock/link result.
        Row(Modifier.fillMaxWidth().height(IntrinsicSize.Min), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            WindAction(stringResource(R.string.action_send_file), stringResource(R.string.home_send_sub), "send", onSendFile?.takeIf { connected != null && state.filesAccepted }, Modifier.weight(1f).fillMaxHeight().testTag("homeSend"))
            WindAction(stringResource(R.string.action_touchpad), stringResource(R.string.home_touchpad_sub), "touch", if (connected != null && touchpadAccepted) { { context.startActivity(android.content.Intent(context, TouchpadActivity::class.java)) } } else null, Modifier.weight(1f).fillMaxHeight().testTag("homeTouchpad"))
        }
        Row(Modifier.fillMaxWidth().height(IntrinsicSize.Min), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            // WN-100: the PC's actions (PcActions.kt).
            WindAction(stringResource(R.string.action_lock_pc), stringResource(R.string.home_lock_sub), "lock", pcActions.lock.takeIf { connected != null }, Modifier.weight(1f).fillMaxHeight().testTag("homeLock"))
            WindAction(stringResource(R.string.action_open_on_pc), stringResource(R.string.home_link_sub), "external", pcActions.openLink.takeIf { connected != null }, Modifier.weight(1f).fillMaxHeight().testTag("homeLink"))
        }
        Text(stringResource(R.string.home_unavailable_note), color = colors.onSurfaceVariant, style = MaterialTheme.typography.bodySmall)
    }
    if (onClipboard != null && onClipboardEnabled != null) ClipboardControls(state, onClipboard, onClipboardEnabled)
    if (history != null) ClipboardHistoryCard(state.clipboardHistory, state.clipboard.enabled, history)
    PcMediaCard() // WN-47: the PC's player; shown only when the PC has one.
    PcVolumeCard() // WN-48: the PC's volume; shown while the PC shares it.
    PcActionsCard(pcActions, connected != null) // WN-100: the outcome, the link box and the PC's own actions.
    if (known) ReceiveFilesControls()
    Text(stringResource(R.string.home_recent), style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
    WindCard(Modifier.fillMaxWidth().testTag("recentTransfers")) {
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            WindIcon("file")
            Column(Modifier.weight(1f)) {
                Text(stringResource(R.string.home_no_transfers), style = MaterialTheme.typography.labelLarge)
                Text(stringResource(R.string.home_transfers_here), color = colors.onSurfaceVariant, style = MaterialTheme.typography.bodySmall)
            }
        }
    }
    if (!state.ready && state.message.isNotEmpty()) Text(state.message, color = colors.error)
    if (state.skippedRecords > 0uL) Text(stringResource(R.string.pairing_skipped, state.skippedRecords.toString()), modifier = Modifier.testTag("pairingStorageWarning"), style = MaterialTheme.typography.bodySmall)
    if (BuildConfig.DEBUG) WindButton(stringResource(R.string.home_diagnostics), onDiagnostics, kind = ButtonKind.Quiet)
}

@Composable
@OptIn(ExperimentalMaterial3Api::class)
fun ClipboardControls(state: PairingUi, onSend: () -> Unit, onEnabled: (Boolean) -> Unit, onImageLimit: ((Int) -> Unit)? = null) {
    WindCard(Modifier.fillMaxWidth().testTag("clipboardCard")) {
        Text(stringResource(R.string.clipboard_shared), style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
        WindSwitch(stringResource(R.string.clip_sync_auto_label), state.clipboard.enabled, onEnabled, Modifier.testTag("clipboardEnabled"))
        // WN-65: the switch already says whether sync is on; the line only shows what happened since.
        // WN-153: by a coded flag, not by comparing text, so a language change can't bring the line back.
        state.clipboard.eventStatus?.let { Text(it, style = MaterialTheme.typography.bodySmall, modifier = Modifier.testTag("clipboardStatus")) }
        Text(stringResource(R.string.clipboard_about), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        if (onImageLimit != null) {
            // WN-36: images over this size are neither sent nor taken.
            Text(stringResource(R.string.clipboard_image_max), style = MaterialTheme.typography.bodyMedium)
            val limits = listOf(5, 16, 50)
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().testTag("clipboardImageLimit")) {
                limits.forEachIndexed { index, mb ->
                    SegmentedButton(selected = state.clipboard.imageLimitMb == mb, onClick = { onImageLimit(mb) },
                        shape = SegmentedButtonDefaults.itemShape(index, limits.size), modifier = Modifier.testTag("imageLimit$mb")) { Text(stringResource(R.string.size_mb, mb)) }
                }
            }
        }
        WindButton(stringResource(R.string.send_clipboard), onSend, Modifier.fillMaxWidth().testTag("sendClipboard"), enabled = state.connection is ConnectionState.Connected && !state.clipboard.busy, kind = ButtonKind.Secondary)
    }
}
