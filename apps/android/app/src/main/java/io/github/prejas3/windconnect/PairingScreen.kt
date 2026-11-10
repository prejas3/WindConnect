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
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp).testTag("pairingScreen"), verticalArrangement = Arrangement.spacedBy(18.dp)) {
        when (state.screen) {
            "home" -> {
                Text("Połącz z komputerem", style = MaterialTheme.typography.headlineSmall)
                Text("Otwórz WindConnect na komputerze i wyświetl kod QR. Oba urządzenia powinny być w tej samej sieci Wi-Fi.")
                WindCard(Modifier.fillMaxWidth()) {
                    Text("Twój komputer pod ręką", style = MaterialTheme.typography.titleMedium)
                    Text("Zeskanuj kod, sprawdź urządzenie i potwierdź parowanie.")
                    WindButton("Zeskanuj kod QR", model::scan, Modifier.testTag("scanQr"), enabled = state.ready)
                }
                Text(state.message.ifEmpty { if (state.ready) "Gotowy do parowania" else "Przygotowywanie bezpiecznego połączenia…" }, modifier = Modifier.testTag("pairingStatus"))
                BackgroundControls(state.hasPairedDevice)
                if (state.skippedRecords > 0uL) Text("Pominięto uszkodzone wpisy sparowania: ${state.skippedRecords}. Poprawne urządzenia są nadal dostępne.", modifier = Modifier.testTag("pairingStorageWarning"), style = MaterialTheme.typography.bodySmall)
                if (BuildConfig.DEBUG) WindButton("Test połączenia z PC", { diagnostics = true }, kind = ButtonKind.Quiet)
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
            "paired" -> {
                Text("Urządzenia sparowane", style = MaterialTheme.typography.headlineSmall, modifier = Modifier.testTag("pairingSuccess"))
                Text(state.name, style = MaterialTheme.typography.titleLarge)
                Text("Klucz komputera został bezpiecznie zapamiętany na tym telefonie.")
                Text(state.message, modifier = Modifier.testTag("pairedConnectionStatus"))
                BackgroundControls(state.hasPairedDevice)
                WindButton("Gotowe", model::cancel, Modifier.testTag("pairingDone"))
            }
            else -> {
                Text("Nie można sparować", style = MaterialTheme.typography.headlineSmall)
                Text(state.message, modifier = Modifier.testTag("pairingError"), color = MaterialTheme.colorScheme.error)
                WindButton("Zeskanuj nowy kod", model::scan)
                WindButton("Wróć", model::cancel, Modifier.testTag("cancelPairing"), kind = ButtonKind.Secondary)
            }
        }
    }
}
