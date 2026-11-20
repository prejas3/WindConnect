package io.github.prejas3.windconnect

import io.github.prejas3.windconnect.core.ConnectionState
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.heading

import androidx.compose.ui.res.stringResource
import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.Settings
import io.github.prejas3.windconnect.core.declaresPermission
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import io.github.prejas3.windconnect.ui.*

@Composable
fun PairingScreen(model: PairingViewModel, diagnostic: @Composable () -> Unit) {
    val state by model.ui.collectAsStateWithLifecycle()
    val context = LocalContext.current
    var hasCamera by remember { mutableStateOf(context.checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) }
    var denied by remember { mutableStateOf(false) }
    var diagnostics by remember { mutableStateOf(false) }
    val scroll = rememberScrollState()
    LaunchedEffect(state.screen) { if (state.screen != "home") diagnostics = false; scroll.scrollTo(0) }
    val lifecycle = LocalLifecycleOwner.current
    DisposableEffect(lifecycle) {
        val observer = object : DefaultLifecycleObserver {
            override fun onResume(owner: LifecycleOwner) {
                hasCamera = context.checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED
            }
        }
        lifecycle.lifecycle.addObserver(observer)
        onDispose { lifecycle.lifecycle.removeObserver(observer) }
    }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted -> hasCamera = granted; denied = !granted }
    BackHandler(state.screen != "home" || diagnostics) { if (diagnostics) diagnostics = false else model.cancel() }
    if (diagnostics && state.screen == "home") { diagnostic(); return }
    Column(Modifier.fillMaxSize().verticalScroll(scroll).padding(24.dp).testTag("pairingScreen"), verticalArrangement = Arrangement.spacedBy(18.dp)) {
        when (state.screen) {
            "home" -> {
                // WC-34: the Welcome card explained the camera, so the system asks right after the tap.
                val scan = { if (!hasCamera) permission.launch(Manifest.permission.CAMERA); model.scan() }
                HomeScreen(state, scan, onDiagnostics = { diagnostics = true }, onNearby = model::nearby, onClipboard = model::sendClipboard, onClipboardEnabled = model::setClipboardEnabled,
                    onSendFile = { context.startActivity(Intent(context, ShareFilesActivity::class.java).setAction(ShareFilesActivity.PICK)) }, history = model.clipboardHistoryActions)
            }
            "devices" -> DevicesScreen(state, onBack = model::cancel, onConnect = model::connectDevice, onRename = model::renameDevice, onUnpair = model::unpairDevice)
            "settings" -> {
                // WN-130: four groups of short rows instead of fourteen same-looking cards; details under "More" in each row.
                WindBackHeader(stringResource(R.string.settings), model::cancel, "closeSettings")
                WindSection(stringResource(R.string.group_computer)) {
                    if (state.devices.isNotEmpty()) {
                        val connectedName = state.devices.firstOrNull { (state.connection as? ConnectionState.Connected)?.fingerprint == it.fingerprint }?.let { computerLabel(it.name) }
                        WindSettingRow("pc", stringResource(R.string.devices_paired),
                            connectedName?.let { stringResource(R.string.devices_summary_one, it) } ?: stringResource(R.string.devices_summary_count, state.devices.size),
                            switchTag = "openDevices", onClick = model::devices)
                        WindDivider()
                    }
                    // WN-160: one way in; the QR screen itself offers pairing without the camera.
                    WindSettingRow("plus", stringResource(R.string.pair_another), stringResource(R.string.sum_pair), switchTag = "scanQr", onClick = model::scan, enabled = state.ready)
                }
                WindSection(stringResource(R.string.group_phone_on_pc)) {
                    NotifyControls()
                    WindDivider()
                    // WN-124: the Play build declares no SMS / Call Log permissions, so these settings aren't there.
                    if (context.declaresPermission(Manifest.permission.READ_SMS)) { SmsControls(); WindDivider() }
                    if (context.declaresPermission(Manifest.permission.READ_CALL_LOG)) { CallControls(); WindDivider() }
                    GalleryControls()
                    WindDivider()
                    MediaControls()
                }
                WindSection(stringResource(R.string.group_pc_on_phone)) {
                    PcMediaControls()
                    WindDivider()
                    LinkControls()
                    WindDivider()
                    FindControls()
                    WindDivider()
                    ReceiveSettingRow()
                }
                WindSection(stringResource(R.string.group_shortcuts)) {
                    ClipboardSettingRows(state, model::setClipboardEnabled, model::setClipboardImageLimit)
                    WindDivider()
                    BackgroundControls(state.hasPairedDevice)
                    WindDivider()
                    ClipboardTileControls()
                    WindDivider()
                    LauncherShortcutControls()
                    WindDivider()
                    WidgetControls()
                    WindDivider()
                    RelayControls()
                }
            }
            "scanner" -> {
                Text(stringResource(R.string.scan_title), style = MaterialTheme.typography.headlineSmall, modifier = Modifier.semantics { heading() })
                Text(stringResource(R.string.scan_about))
                if (hasCamera) {
                    QrCamera(Modifier.fillMaxWidth().height(320.dp).testTag("cameraPreview"), model::offerLink, model::cameraError)
                } else {
                    Text(if (denied) stringResource(R.string.scan_denied) else stringResource(R.string.scan_consent))
                    WindButton(stringResource(R.string.camera_allow), { permission.launch(Manifest.permission.CAMERA) }, Modifier.testTag("cameraPermission"))
                    if (denied) WindButton(stringResource(R.string.permission_settings), { context.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${context.packageName}"))) }, kind = ButtonKind.Secondary)
                }
                WindButton(stringResource(R.string.pair_without_camera), model::nearby, Modifier.fillMaxWidth().testTag("pairNearby"), enabled = state.ready, kind = ButtonKind.Secondary)
                WindButton(stringResource(R.string.cancel), model::cancel, Modifier.fillMaxWidth().testTag("cancelPairing"), kind = ButtonKind.Quiet)
            }
            "confirm", "pairing" -> {
                Text(stringResource(R.string.confirm_title), style = MaterialTheme.typography.headlineSmall, modifier = Modifier.semantics { heading() })
                WindCard(Modifier.fillMaxWidth()) {
                    Text(state.name, style = MaterialTheme.typography.titleLarge, modifier = Modifier.testTag("pairingPeer"))
                    Text(stringResource(R.string.confirm_fingerprint), style = MaterialTheme.typography.labelMedium)
                    Text(state.fingerprint, style = MaterialTheme.typography.bodySmall, modifier = Modifier.testTag("pairingFingerprint"))
                }
                Text(stringResource(R.string.confirm_about))
                WindButton(if (state.screen == "pairing") stringResource(R.string.pairing_progress) else stringResource(R.string.pair_devices), model::confirm, Modifier.fillMaxWidth().testTag("confirmPairing"), enabled = state.ready && state.screen == "confirm")
                if (state.screen == "confirm") WindButton(stringResource(R.string.cancel), model::cancel, Modifier.testTag("cancelPairing"), kind = ButtonKind.Secondary)
                else Text(stringResource(R.string.pairing_verifying), modifier = Modifier.testTag("pairingProgress"))
            }
            "nearby" -> {
                Text(stringResource(R.string.nearby_title), style = MaterialTheme.typography.headlineSmall, modifier = Modifier.semantics { heading() })
                Text(stringResource(R.string.nearby_about))
                if (state.nearby.isEmpty()) Text(stringResource(R.string.nearby_searching), modifier = Modifier.testTag("nearbyEmpty"))
                state.nearby.forEach { device ->
                    WindCard(Modifier.fillMaxWidth().testTag("nearbyDevice")) {
                        Text(stringResource(R.string.nearby_ready), style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
                        Text(shortFingerprint(device.fingerprint), style = MaterialTheme.typography.bodySmall)
                        WindButton(stringResource(R.string.pair), { model.pairWith(device) }, Modifier.fillMaxWidth())
                    }
                }
                Text(stringResource(R.string.nearby_note), style = MaterialTheme.typography.bodySmall)
                WindButton(stringResource(R.string.cancel), model::cancel, Modifier.testTag("cancelPairing"), kind = ButtonKind.Secondary)
            }
            "sas-connecting" -> {
                Text(stringResource(R.string.sas_connecting), style = MaterialTheme.typography.headlineSmall, modifier = Modifier.semantics { heading() })
                Text(stringResource(R.string.sas_soon))
                WindButton(stringResource(R.string.cancel), model::cancel, Modifier.testTag("cancelPairing"), kind = ButtonKind.Secondary)
            }
            "sas-code", "sas-rejected", "sas-confirming" -> {
                Text(stringResource(R.string.sas_compare), style = MaterialTheme.typography.headlineSmall, modifier = Modifier.semantics { heading() })
                WindCard(Modifier.fillMaxWidth()) {
                    Text(state.name, style = MaterialTheme.typography.titleMedium, modifier = Modifier.testTag("pairingPeer"))
                    Text(state.code, style = MaterialTheme.typography.displayMedium, modifier = Modifier.testTag("sasCode"))
                }
                Text(stringResource(R.string.sas_about))
                WindButton(stringResource(R.string.sas_same), { model.decide(true) }, Modifier.fillMaxWidth().testTag("sasSame"), enabled = state.screen == "sas-code")
                WindButton(stringResource(R.string.sas_different), { model.decide(false) }, Modifier.fillMaxWidth().testTag("sasDifferent"), enabled = state.screen == "sas-code", kind = ButtonKind.Secondary)
                if (state.screen == "sas-confirming") Text(stringResource(R.string.sas_waiting), modifier = Modifier.testTag("pairingProgress"))
            }
            "paired" -> DoneScreen(state.name, state.message, onDone = model::cancel)
            else -> {
                Text(stringResource(R.string.pair_failed), style = MaterialTheme.typography.headlineSmall, modifier = Modifier.semantics { heading() })
                Text(state.message, modifier = Modifier.testTag("pairingError"), color = MaterialTheme.colorScheme.error)
                WindButton(stringResource(R.string.scan_new), model::scan)
                WindButton(stringResource(R.string.go_back), model::cancel, Modifier.testTag("cancelPairing"), kind = ButtonKind.Secondary)
            }
        }
    }
}
