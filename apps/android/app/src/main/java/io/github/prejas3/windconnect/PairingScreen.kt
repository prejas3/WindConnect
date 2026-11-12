package io.github.prejas3.windconnect

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.Settings
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
                HomeScreen(state, scan, onDiagnostics = { diagnostics = true }, onNearby = model::nearby, onClipboard = model::sendClipboard, onClipboardEnabled = model::setClipboardEnabled)
                if (state.devices.isNotEmpty()) WindButton("Sparowane komputery", model::devices, Modifier.fillMaxWidth().testTag("openDevices"), kind = ButtonKind.Secondary)
            }
            "devices" -> DevicesScreen(state, onBack = model::cancel, onConnect = model::connectDevice, onRename = model::renameDevice, onUnpair = model::unpairDevice)
            "settings" -> {
                Text("Ustawienia", style = MaterialTheme.typography.headlineSmall)
                WindButton("Powrót", model::cancel, Modifier.testTag("closeSettings"), kind = ButtonKind.Quiet)
                ClipboardTileControls()
                LauncherShortcutControls()
                WidgetControls()
                BackgroundControls(state.hasPairedDevice)
                FindControls()
                MediaControls()
                ClipboardControls(state, model::sendClipboard, model::setClipboardEnabled, model::setClipboardImageLimit)
                if (state.devices.isNotEmpty()) WindButton("Sparowane komputery", model::devices, Modifier.testTag("openDevices"), kind = ButtonKind.Secondary)
                WindButton("Sparuj kolejny komputer", model::scan, Modifier.testTag("scanQr"), enabled = state.ready, kind = ButtonKind.Secondary)
                WindButton("Paruj bez aparatu", model::nearby, Modifier.testTag("pairNearby"), enabled = state.ready, kind = ButtonKind.Secondary)
            }
            "scanner" -> {
                Text("Zeskanuj kod QR", style = MaterialTheme.typography.headlineSmall)
                Text("Skieruj aparat na kod w WindConnect na komputerze. Obraz jest analizowany wyłącznie na telefonie.")
                if (hasCamera) {
                    QrCamera(Modifier.fillMaxWidth().height(320.dp).testTag("cameraPreview"), model::offerLink, model::cameraError)
                } else {
                    Text(if (denied) "Brak dostępu do aparatu. Możesz zezwolić w ustawieniach albo otworzyć kod aparatem systemowym." else "Aparat zostanie włączony dopiero po udzieleniu zgody.")
                    WindButton("Zezwól na aparat", { permission.launch(Manifest.permission.CAMERA) }, Modifier.testTag("cameraPermission"))
                    if (denied) WindButton("Ustawienia uprawnień", { context.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${context.packageName}"))) }, kind = ButtonKind.Secondary)
                }
                WindButton("Anuluj", model::cancel, Modifier.testTag("cancelPairing"), kind = ButtonKind.Secondary)
            }
            "confirm", "pairing" -> {
                Text("Sparować z komputerem?", style = MaterialTheme.typography.headlineSmall)
                WindCard(Modifier.fillMaxWidth()) {
                    Text(state.name, style = MaterialTheme.typography.titleLarge, modifier = Modifier.testTag("pairingPeer"))
                    Text("Odcisk klucza komputera", style = MaterialTheme.typography.labelMedium)
                    Text(state.fingerprint, style = MaterialTheme.typography.bodySmall, modifier = Modifier.testTag("pairingFingerprint"))
                }
                Text("Nazwa pochodzi z kodu QR. Potwierdź tylko kod wyświetlony na Twoim komputerze. Samo otwarcie linku nie nadaje urządzeniu zaufania.")
                WindButton(if (state.screen == "pairing") "Parowanie…" else "Sparuj urządzenia", model::confirm, Modifier.fillMaxWidth().testTag("confirmPairing"), enabled = state.ready && state.screen == "confirm")
                if (state.screen == "confirm") WindButton("Anuluj", model::cancel, Modifier.testTag("cancelPairing"), kind = ButtonKind.Secondary)
                else Text("Weryfikowanie jednorazowego kodu. To może potrwać do 30 sekund.", modifier = Modifier.testTag("pairingProgress"))
            }
            "nearby" -> {
                Text("Komputery w pobliżu", style = MaterialTheme.typography.headlineSmall)
                Text("Na komputerze otwórz panel WindConnect i wybierz „Paruj bez aparatu”. Komputer pojawi się tutaj.")
                if (state.nearby.isEmpty()) Text("Szukam w tej sieci Wi-Fi…", modifier = Modifier.testTag("nearbyEmpty"))
                state.nearby.forEach { device ->
                    WindCard(Modifier.fillMaxWidth().testTag("nearbyDevice")) {
                        Text("Komputer gotowy do parowania", style = MaterialTheme.typography.titleMedium)
                        Text(shortFingerprint(device.fingerprint), style = MaterialTheme.typography.bodySmall)
                        WindButton("Paruj", { model.pairWith(device) }, Modifier.fillMaxWidth())
                    }
                }
                Text("Lista pochodzi z sieci i niczego nie potwierdza. O zaufaniu decyduje porównanie kodów na obu ekranach.", style = MaterialTheme.typography.bodySmall)
                WindButton("Anuluj", model::cancel, Modifier.testTag("cancelPairing"), kind = ButtonKind.Secondary)
            }
            "sas-connecting" -> {
                Text("Łączenie z komputerem…", style = MaterialTheme.typography.headlineSmall)
                Text("Za chwilę oba ekrany pokażą ten sam sześciocyfrowy kod.")
                WindButton("Anuluj", model::cancel, Modifier.testTag("cancelPairing"), kind = ButtonKind.Secondary)
            }
            "sas-code", "sas-rejected", "sas-confirming" -> {
                Text("Porównaj kody", style = MaterialTheme.typography.headlineSmall)
                WindCard(Modifier.fillMaxWidth()) {
                    Text(state.name, style = MaterialTheme.typography.titleMedium, modifier = Modifier.testTag("pairingPeer"))
                    Text(state.code, style = MaterialTheme.typography.displayMedium, modifier = Modifier.testTag("sasCode"))
                }
                Text("Sparuj tylko wtedy, gdy komputer pokazuje dokładnie ten sam kod. Inny kod oznacza, że łączysz się z innym urządzeniem.")
                WindButton("Kody są takie same", { model.decide(true) }, Modifier.fillMaxWidth().testTag("sasSame"), enabled = state.screen == "sas-code")
                WindButton("Kody się różnią", { model.decide(false) }, Modifier.fillMaxWidth().testTag("sasDifferent"), enabled = state.screen == "sas-code", kind = ButtonKind.Secondary)
                if (state.screen == "sas-confirming") Text("Czekam na potwierdzenie na komputerze.", modifier = Modifier.testTag("pairingProgress"))
            }
            "paired" -> DoneScreen(state.name, state.message, onDone = model::cancel)
            else -> {
                Text("Nie można sparować", style = MaterialTheme.typography.headlineSmall)
                Text(state.message, modifier = Modifier.testTag("pairingError"), color = MaterialTheme.colorScheme.error)
                WindButton("Zeskanuj nowy kod", model::scan)
                WindButton("Wróć", model::cancel, Modifier.testTag("cancelPairing"), kind = ButtonKind.Secondary)
            }
        }
    }
}
