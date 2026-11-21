package io.github.prejas3.windconnect

import androidx.compose.ui.semantics.heading

import androidx.compose.ui.res.stringResource
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
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
import androidx.compose.runtime.setValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.withFrameNanos
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester

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
    // WN-130: one card for the computer (the strip above it said the same): name, state and route under it, then its actions.
    WindCard(Modifier.fillMaxWidth().testTag("computerCard")) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Box(Modifier.size(44.dp).clip(WindShapes.medium).background(if (connected != null) colors.primaryContainer else colors.surface), contentAlignment = Alignment.Center) { WindIcon("pc") }
            Column(Modifier.weight(1f)) {
                Text(name, modifier = Modifier.testTag("computerName"), style = MaterialTheme.typography.titleLarge, maxLines = 2, overflow = TextOverflow.Ellipsis)
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    Box(Modifier.size(8.dp).clip(WindShapes.full).background(if (connected != null) colors.primary else colors.onSurfaceVariant))
                    val latency = connected?.let { String.format(Locale.getDefault(), "%.1f ms", it.roundTripMicros / 1000.0) }
                    val route = stringResource(if (connected?.route == "relay") R.string.home_via_relay else R.string.home_local_network)
                    val latencyDescription = if (latency != null) stringResource(R.string.home_latency_desc, latency) else status
                    Text(status, style = MaterialTheme.typography.bodyMedium, color = colors.onSurfaceVariant, modifier = Modifier.testTag("pairingStatus"))
                    if (connected != null) Text("· " + stringResource(R.string.home_latency_line, latency.orEmpty(), route),
                        style = MaterialTheme.typography.bodySmall, color = colors.onSurfaceVariant,
                        modifier = Modifier.testTag("computerLatency").semantics { contentDescription = latencyDescription })
                }
            }
        }
        // WN-160: one line says why the actions are off (the tiles are only dimmed), instead of "Unavailable" on each.
        if (connected == null && state.initialized) Text(stringResource(if (known) R.string.home_actions_offline else R.string.home_actions_unpaired),
            style = MaterialTheme.typography.bodyMedium, color = colors.onSurface, modifier = Modifier.testTag("actionsOffline"))
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
    }
    // WN-130/WN-156: "History" in the clipboard card's header shows the history card (also when empty) and scrolls to it;
    // an empty history otherwise takes no card.
    var historyAsked by remember { mutableIntStateOf(0) }
    val historyView = remember { BringIntoViewRequester() }
    if (onClipboard != null && onClipboardEnabled != null) ClipboardControls(state, onClipboard, onClipboardEnabled, onHistory = history?.let { { historyAsked++ } })
    // WN-169: the card opens (expand), and is scrolled to once it has its full height.
    val reduce = LocalReduceMotion.current
    androidx.compose.animation.AnimatedVisibility(history != null && (state.clipboardHistory.isNotEmpty() || historyAsked > 0), enter = expandIn(reduce), exit = shrinkOut(reduce)) {
        val settled = !transition.isRunning
        LaunchedEffect(historyAsked, settled) { if (historyAsked > 0 && settled) { withFrameNanos { }; historyView.bringIntoView() } }
        if (history != null) Box(Modifier.bringIntoViewRequester(historyView)) {
            ClipboardHistoryCard(state.clipboardHistory, state.clipboard.enabled, history)
        }
    }
    PcMediaCard() // WN-47/48: the PC's player and volume, one card, shown while the PC shares either.
    PcActionsCard(pcActions, connected != null) // WN-100: the outcome, the link box and the PC's own actions.
    // WN-130: received files, newest few (the always-empty "Recent transfers" card is gone).
    if (known) ReceiveFilesControls()
    if (!state.ready && state.message.isNotEmpty()) Text(state.message, color = colors.error)
    if (state.skippedRecords > 0uL) Text(stringResource(R.string.pairing_skipped, state.skippedRecords.toString()), modifier = Modifier.testTag("pairingStorageWarning"), style = MaterialTheme.typography.bodySmall)
    if (BuildConfig.DEBUG) WindButton(stringResource(R.string.home_diagnostics), onDiagnostics, kind = ButtonKind.Quiet)
}

@Composable
fun ClipboardControls(state: PairingUi, onSend: () -> Unit, onEnabled: (Boolean) -> Unit, onHistory: (() -> Unit)? = null) {
    // WN-130 home: the switch, what happened since, and one action; the explanation and image limit are in Settings.
    // WN-160: compact: the switch carries what it covers, the last event is a state with its time, then the button.
    val context = LocalContext.current
    WindCard(Modifier.fillMaxWidth().testTag("clipboardCard")) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(stringResource(R.string.clipboard_shared), style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f).semantics { heading() })
            if (onHistory != null) WindHeaderAction(stringResource(R.string.clipboard_history_link), onHistory, Modifier.testTag("clipboardHistoryLink"))
        }
        WindSwitch(stringResource(R.string.clip_sync_auto_label), state.clipboard.enabled, onEnabled, Modifier.testTag("clipboardEnabled"),
            description = stringResource(R.string.sum_clip_sync))
        // WN-65: the switch already says whether sync is on; the line only shows what happened since.
        // WN-153: by a coded flag, not by comparing text, so a language change can't bring the line back.
        state.clipboard.eventStatus?.let { event ->
            val at = state.clipboard.statusAt
            val line = if (at > 0) stringResource(R.string.clipboard_last, event, android.text.format.DateFormat.getTimeFormat(context).format(java.util.Date(at))) else event
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                WindIcon("clipboard", Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                Text(line, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface, modifier = Modifier.testTag("clipboardStatus"))
            }
        }
        WindButton(stringResource(R.string.send_clipboard_now), onSend, Modifier.fillMaxWidth().testTag("sendClipboard"),
            enabled = state.connection is ConnectionState.Connected && !state.clipboard.busy, kind = ButtonKind.Secondary)
    }
}

/** WN-130 Settings, group Clipboard: automatic sync (the explanation under "More") and the image limit (WN-36). */
@Composable
fun ClipboardSettingRows(state: PairingUi, onEnabled: (Boolean) -> Unit, onImageLimit: (Int) -> Unit) {
    WindSettingRow("clipboard", stringResource(R.string.clip_sync_auto_label), stringResource(R.string.sum_clip_sync), checked = state.clipboard.enabled,
        onCheckedChange = onEnabled, switchTag = "clipboardEnabled", more = { Text(stringResource(R.string.clipboard_about)) })
    WindDivider()
    WindSettingRow("image", stringResource(R.string.clipboard_image_max), actions = {
        // WN-36: images over this size are neither sent nor taken.
        val limits = listOf(5, 16, 50)
        WindSegmented(limits, state.clipboard.imageLimitMb, { stringResource(R.string.size_mb, it) }, onImageLimit,
            Modifier.testTag("clipboardImageLimit"), tag = { "imageLimit$it" })
    })
}
