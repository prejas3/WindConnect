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
import androidx.activity.BackEventCompat
import androidx.activity.compose.PredictiveBackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.ui.Alignment
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalDensity
import kotlin.coroutines.cancellation.CancellationException
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.background
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
    LaunchedEffect(state.screen) { if (state.screen != "home") diagnostics = false }
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
    // WN-169: predictive back. While the person swipes, the screen shrinks and moves towards the swipe (a preview of
    // leaving it); letting go goes back, cancelling restores it. Below Android 13 back simply goes back.
    val reduce = LocalReduceMotion.current
    val density = LocalDensity.current
    var backProgress by remember { mutableFloatStateOf(0f) }
    var backEdge by remember { mutableIntStateOf(BackEventCompat.EDGE_LEFT) }
    PredictiveBackHandler(state.screen != "home" || diagnostics) { events ->
        try {
            events.collect { backProgress = it.progress; backEdge = it.swipeEdge }
            backProgress = 0f
            if (diagnostics) diagnostics = false else model.cancel()
        } catch (cancelled: CancellationException) {
            backProgress = 0f
            throw cancelled
        }
    }
    val page = if (diagnostics && state.screen == "home") "diagnostics" else state.screen
    // WN-169: steps and screens change on the M3 shared X axis (deeper = forward), none when animations are off.
    AnimatedContent(page, Modifier.fillMaxSize().graphicsLayer {
        if (!reduce && backProgress > 0f) {
            val scale = 1f - .1f * backProgress
            scaleX = scale; scaleY = scale
            translationX = (if (backEdge == BackEventCompat.EDGE_LEFT) 1 else -1) * backProgress * 16.dp.toPx()
        }
    }, transitionSpec = { sharedAxisX(pairingDepth(targetState) >= pairingDepth(initialState), reduce, density) },
        contentKey = ::pairingStep, label = "pairingStep") { screen ->
    if (screen == "diagnostics") { diagnostic(); return@AnimatedContent }
    // WN-170: every screen but home and the final "Done" is a WindScreen: the pinned top bar with back (or close for a
    // pairing step, which cancels it), content at most 640 dp wide.
    val cancel = stringResource(R.string.cancel)
    val back = stringResource(R.string.back)
    val step = Modifier.testTag("pairingScreen")
    when (screen) {
        "home" -> HomePage {
            // WC-34: the Welcome card explained the camera, so the system asks right after the tap.
            val scan = { if (!hasCamera) permission.launch(Manifest.permission.CAMERA); model.scan() }
            HomeScreen(state, scan, onDiagnostics = { diagnostics = true }, onNearby = model::nearby, onClipboard = model::sendClipboard, onClipboardEnabled = model::setClipboardEnabled,
                onSendFile = { context.startActivity(Intent(context, ShareFilesActivity::class.java).setAction(ShareFilesActivity.PICK)) }, history = model.clipboardHistoryActions)
        }
        "devices" -> WindScreen(stringResource(R.string.devices_paired), model::cancel, back, "closeDevices", step) {
            DevicesScreen(state, onBack = model::cancel, onConnect = model::connectDevice, onRename = model::renameDevice, onUnpair = model::unpairDevice)
        }
        "settings" -> {
            // WN-130: four groups of short rows instead of fourteen same-looking cards; details under (i) in each row.
            val groups: List<Pair<String, @Composable ColumnScope.() -> Unit>> = listOf(
                stringResource(R.string.group_computer) to {
                    if (state.devices.isNotEmpty()) {
                        val connectedName = state.devices.firstOrNull { (state.connection as? ConnectionState.Connected)?.fingerprint == it.fingerprint }?.let { computerLabel(it.name) }
                        WindSettingRow("pc", stringResource(R.string.devices_paired),
                            connectedName?.let { stringResource(R.string.devices_summary_one, it) } ?: stringResource(R.string.devices_summary_count, state.devices.size),
                            switchTag = "openDevices", onClick = model::devices)
                        WindDivider()
                    }
                    // WN-160: one way in; the QR screen itself offers pairing without the camera.
                    WindSettingRow("plus", stringResource(R.string.pair_another), stringResource(R.string.sum_pair), switchTag = "scanQr", onClick = model::scan, enabled = state.ready)
                },
                stringResource(R.string.group_phone_on_pc) to {
                    NotifyControls()
                    WindDivider()
                    // WN-124: the Play build declares no SMS / Call Log permissions, so these settings aren't there.
                    if (context.declaresPermission(Manifest.permission.READ_SMS)) { SmsControls(); WindDivider() }
                    if (context.declaresPermission(Manifest.permission.READ_CALL_LOG)) { CallControls(); WindDivider() }
                    GalleryControls()
                    WindDivider()
                    MediaControls()
                },
                stringResource(R.string.group_pc_on_phone) to {
                    PcMediaControls()
                    WindDivider()
                    LinkControls()
                    WindDivider()
                    FindControls()
                    WindDivider()
                    ReceiveSettingRow()
                },
                stringResource(R.string.group_shortcuts) to {
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
                },
            )
            SettingsPage(groups, model::cancel, step)
        }
        "scanner" -> WindScreen(stringResource(R.string.scan_title), model::cancel, cancel, "cancelPairing", step, close = true) {
            Text(stringResource(R.string.scan_about))
            if (hasCamera) {
                // WN-171: the preview rounded (28 dp), a viewfinder over a dimmed surround, the hint on the preview itself.
                Box(Modifier.fillMaxWidth().height(320.dp).clip(WindShapes.extraLarge)) {
                    QrCamera(Modifier.matchParentSize().testTag("cameraPreview"), model::offerLink, model::cameraError)
                    QrViewfinder(Modifier.matchParentSize())
                    Text(stringResource(R.string.scan_aim), style = MaterialTheme.typography.bodyMedium, color = androidx.compose.ui.graphics.Color.White,
                        modifier = Modifier.align(Alignment.BottomCenter).padding(16.dp).clip(WindShapes.full)
                            .background(androidx.compose.ui.graphics.Color.Black.copy(alpha = .55f)).padding(horizontal = 12.dp, vertical = 6.dp).testTag("scanHint"))
                }
            } else {
                Text(if (denied) stringResource(R.string.scan_denied) else stringResource(R.string.scan_consent))
                WindButton(stringResource(R.string.camera_allow), { permission.launch(Manifest.permission.CAMERA) }, Modifier.testTag("cameraPermission"))
                if (denied) WindButton(stringResource(R.string.permission_settings), { context.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${context.packageName}"))) }, kind = ButtonKind.Secondary)
            }
            WindButton(stringResource(R.string.pair_without_camera), model::nearby, Modifier.fillMaxWidth().testTag("pairNearby"), enabled = state.ready, kind = ButtonKind.Secondary)
        }
        "confirm", "pairing" -> WindScreen(stringResource(R.string.confirm_title), model::cancel, cancel, "cancelPairing", step, close = true) {
            WindCard(Modifier.fillMaxWidth()) {
                Text(state.name, style = MaterialTheme.typography.titleLarge, modifier = Modifier.testTag("pairingPeer"))
                Text(stringResource(R.string.confirm_fingerprint), style = MaterialTheme.typography.labelMedium)
                // WN-171: the key's fingerprint in monospace groups of four, easy to compare by eye.
                Text(groupFingerprint(state.fingerprint), style = MaterialTheme.typography.bodyMedium.copy(fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace, letterSpacing = androidx.compose.ui.unit.TextUnit(0.5f, androidx.compose.ui.unit.TextUnitType.Sp)),
                    modifier = Modifier.testTag("pairingFingerprint"))
            }
            Text(stringResource(R.string.confirm_about))
            WindButton(if (state.screen == "pairing") stringResource(R.string.pairing_progress) else stringResource(R.string.pair_devices), model::confirm, Modifier.fillMaxWidth().testTag("confirmPairing"), enabled = state.ready && state.screen == "confirm")
            if (state.screen == "pairing") Waiting(stringResource(R.string.pairing_verifying), "pairingProgress")
        }
        "nearby" -> WindScreen(stringResource(R.string.nearby_title), model::cancel, cancel, "cancelPairing", step, close = true) {
            Text(stringResource(R.string.nearby_about))
            if (state.nearby.isEmpty()) Waiting(stringResource(R.string.nearby_searching), "nearbyEmpty")
            state.nearby.forEach { device ->
                WindCard(Modifier.fillMaxWidth().testTag("nearbyDevice")) {
                    Text(stringResource(R.string.nearby_ready), style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
                    Text(shortFingerprint(device.fingerprint), style = MaterialTheme.typography.bodySmall)
                    WindButton(stringResource(R.string.pair), { model.pairWith(device) }, Modifier.fillMaxWidth())
                }
            }
            Text(stringResource(R.string.nearby_note), style = MaterialTheme.typography.bodySmall)
        }
        "sas-connecting" -> WindScreen(stringResource(R.string.sas_connecting), model::cancel, cancel, "cancelPairing", step, close = true) {
            Waiting(stringResource(R.string.sas_soon), "sasConnecting")
        }
        "sas-code", "sas-rejected", "sas-confirming" -> WindScreen(stringResource(R.string.sas_compare), model::cancel, cancel, "cancelPairing", step, close = true) {
            WindCard(Modifier.fillMaxWidth()) {
                Text(state.name, style = MaterialTheme.typography.titleMedium, modifier = Modifier.testTag("pairingPeer"))
                // WN-171: the code in groups of three with tabular digits (each digit the same width, as on the PC).
                Text(groupCode(state.code), style = MaterialTheme.typography.displayMedium.copy(fontFeatureSettings = "tnum"), modifier = Modifier.testTag("sasCode"))
            }
            Text(stringResource(R.string.sas_about))
            WindButton(stringResource(R.string.sas_same), { model.decide(true) }, Modifier.fillMaxWidth().testTag("sasSame"), enabled = state.screen == "sas-code")
            WindButton(stringResource(R.string.sas_different), { model.decide(false) }, Modifier.fillMaxWidth().testTag("sasDifferent"), enabled = state.screen == "sas-code", kind = ButtonKind.Secondary)
            if (state.screen == "sas-confirming") Waiting(stringResource(R.string.sas_waiting), "pairingProgress")
        }
        "paired" -> HomePage { DoneScreen(state.name, state.message, onDone = model::cancel) }
        else -> WindScreen(stringResource(R.string.pair_failed), model::cancel, back, "cancelPairing", step) {
            Text(state.message, modifier = Modifier.testTag("pairingError"), color = MaterialTheme.colorScheme.error)
            WindButton(stringResource(R.string.scan_new), model::scan, Modifier.fillMaxWidth())
        }
    }
    }
}

/** WN-169: how deep a pairing screen sits, so a step further in slides forward and a step back slides back. */
private fun pairingDepth(screen: String): Int = when (screen) {
    "home", "diagnostics" -> 0
    "settings", "devices", "scanner", "nearby" -> 1
    "confirm", "pairing", "sas-connecting" -> 2
    "sas-code", "sas-rejected", "sas-confirming" -> 3
    else -> 4
}

/** WN-169: screens that are one step in different states (confirm → pairing) change in place, without a transition. */
private fun pairingStep(screen: String): String = when (screen) {
    "confirm", "pairing" -> "confirm"
    "sas-code", "sas-rejected", "sas-confirming" -> "sas"
    else -> screen
}

/** WN-169: a wait in pairing (searching, connecting, verifying): the circular progress beside its text. */
@Composable
private fun Waiting(text: String, tag: String) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        WindCircularProgress(diameter = 24.dp)
        Text(text, modifier = Modifier.testTag(tag).weight(1f))
    }
}

/** WN-170: home and the final "Done": no top bar of their own (home has the app's), the same 640 dp centred column. */
@Composable
private fun HomePage(content: @Composable ColumnScope.() -> Unit) {
    Box(Modifier.fillMaxSize().verticalScroll(rememberScrollState()), contentAlignment = Alignment.TopCenter) {
        Column(Modifier.widthIn(max = MAX_CONTENT_WIDTH).fillMaxWidth().padding(24.dp).testTag("pairingScreen"), verticalArrangement = Arrangement.spacedBy(18.dp), content = content)
    }
}

/**
 * WN-170: Settings. Compact and medium windows: the groups one under another. Expanded (≥ 840 dp): a list of the groups
 * at the start and the chosen group in a panel beside it.
 */
@Composable
internal fun SettingsPage(groups: List<Pair<String, @Composable ColumnScope.() -> Unit>>, onBack: () -> Unit, modifier: Modifier = Modifier) {
    val title = stringResource(R.string.settings)
    val back = stringResource(R.string.back)
    if (!expandedWidth()) {
        WindScreen(title, onBack, back, "closeSettings", modifier) { groups.forEach { (name, rows) -> WindSection(name) { rows() } } }
        return
    }
    val colors = MaterialTheme.colorScheme
    var chosen by rememberSaveable { mutableIntStateOf(0) }
    val panel = rememberScrollState()
    Column(modifier.fillMaxSize()) {
        WindTopBar(title, "back", back, onBack, "closeSettings", scrolled = panel.value > 0)
        Row(Modifier.weight(1f).fillMaxWidth().padding(horizontal = 24.dp), horizontalArrangement = Arrangement.spacedBy(24.dp)) {
            Column(Modifier.width(280.dp).fillMaxHeight().verticalScroll(rememberScrollState()).padding(vertical = 8.dp).testTag("settingsList"),
                verticalArrangement = Arrangement.spacedBy(4.dp)) {
                groups.forEachIndexed { index, (name, _) ->
                    val on = index == chosen
                    Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).windFocus(WindShapes.full).clip(WindShapes.full)
                        .background(if (on) colors.secondaryContainer else androidx.compose.ui.graphics.Color.Transparent)
                        .selectable(selected = on, role = Role.Tab) { chosen = index }.testTag("settingsGroup$index").padding(horizontal = 16.dp),
                        verticalAlignment = Alignment.CenterVertically) {
                        Text(name, style = MaterialTheme.typography.labelLarge, color = if (on) colors.onSecondaryContainer else colors.onSurfaceVariant)
                    }
                }
            }
            Column(Modifier.weight(1f).fillMaxHeight().verticalScroll(panel).padding(vertical = 8.dp).testTag("settingsPanel"), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                val (name, rows) = groups[chosen.coerceIn(groups.indices)]
                Box(Modifier.widthIn(max = MAX_CONTENT_WIDTH)) { WindSection(name) { rows() } }
            }
        }
    }
}

/** WN-171: "123456" → "123 456"; other shapes are shown as they are. */
internal fun groupCode(code: String): String = if (code.length > 4 && code.all { it.isDigit() }) code.chunked(3).joinToString(" ") else code

/** WN-171: a fingerprint without its own spacing in groups of four (as the PC shows it). */
internal fun groupFingerprint(fingerprint: String): String =
    if (fingerprint.any { it.isWhitespace() || it == ':' }) fingerprint else fingerprint.chunked(4).joinToString(" ")

/** WN-171: the QR viewfinder: the surround dimmed, a clear rounded square with white corner marks in the middle. */
@Composable
private fun QrViewfinder(modifier: Modifier) {
    androidx.compose.foundation.Canvas(modifier) {
        val side = minOf(size.width, size.height) * .66f
        val left = (size.width - side) / 2; val top = (size.height - side) / 2
        val radius = 20.dp.toPx()
        val hole = androidx.compose.ui.geometry.RoundRect(left, top, left + side, top + side, androidx.compose.ui.geometry.CornerRadius(radius))
        val dim = androidx.compose.ui.graphics.Path().apply {
            fillType = androidx.compose.ui.graphics.PathFillType.EvenOdd
            addRect(androidx.compose.ui.geometry.Rect(0f, 0f, size.width, size.height)); addRoundRect(hole)
        }
        drawPath(dim, androidx.compose.ui.graphics.Color.Black.copy(alpha = .45f))
        val mark = 28.dp.toPx(); val stroke = androidx.compose.ui.graphics.drawscope.Stroke(4.dp.toPx(), cap = androidx.compose.ui.graphics.StrokeCap.Round)
        for ((x, y, dx, dy) in listOf(listOf(left, top, 1f, 1f), listOf(left + side, top, -1f, 1f), listOf(left, top + side, 1f, -1f), listOf(left + side, top + side, -1f, -1f))) {
            val corner = androidx.compose.ui.graphics.Path().apply {
                moveTo(x, y + dy * mark); lineTo(x, y + dy * radius)
                quadraticTo(x, y, x + dx * radius, y); lineTo(x + dx * mark, y)
            }
            drawPath(corner, androidx.compose.ui.graphics.Color.White, style = stroke)
        }
    }
}
