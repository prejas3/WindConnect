package io.github.prejas3.windconnect.ui

import android.os.Build
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import androidx.compose.ui.text.style.TextOverflow
import io.github.prejas3.windconnect.BuildConfig

@Composable
fun WindApp(forceContent: Boolean = false, connection: @Composable () -> Unit) {
    val preferences = LocalContext.current.getSharedPreferences("appearance", 0)
    var modeName by rememberSaveable { mutableStateOf(preferences.getString("theme", ThemeMode.System.name) ?: ThemeMode.System.name) }
    var dynamic by rememberSaveable { mutableStateOf(preferences.getBoolean("dynamic", true)) }
    var preview by rememberSaveable { mutableStateOf(false) }
    LaunchedEffect(forceContent) { if (forceContent) preview = false }
    val mode = ThemeMode.entries.firstOrNull { it.name == modeName } ?: ThemeMode.System
    WindConnectTheme(mode, dynamic) {
        Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).safeDrawingPadding().imePadding()) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 8.dp), horizontalArrangement = Arrangement.SpaceBetween) {
                Text("WindConnect", modifier = Modifier.weight(1f).padding(end = 12.dp, top = 10.dp, bottom = 10.dp), style = MaterialTheme.typography.titleLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (BuildConfig.DEBUG && !forceContent) WindButton(if (preview) "Połączenie" else "Podgląd", onClick = { preview = !preview }, modifier = Modifier.testTag("previewToggle"), kind = ButtonKind.Quiet)
            }
            Box(Modifier.weight(1f)) {
                if (BuildConfig.DEBUG && preview && !forceContent) ComponentPreview(
                    mode, dynamic,
                    onMode = { modeName = it.name; preferences.edit().putString("theme", it.name).apply() },
                    onDynamic = { dynamic = it; preferences.edit().putBoolean("dynamic", it).apply() },
                ) else connection()
            }
        }
    }
}

@Composable
private fun ComponentPreview(mode: ThemeMode, dynamic: Boolean, onMode: (ThemeMode) -> Unit, onDynamic: (Boolean) -> Unit) {
    var clipboard by rememberSaveable { mutableStateOf(true) }
    var name by rememberSaveable { mutableStateOf("") }
    var validated by rememberSaveable { mutableStateOf(false) }
    var feedback by rememberSaveable { mutableStateOf("Wszystkie dane i akcje są przykładowe.") }
    var complete by rememberSaveable { mutableStateOf(false) }
    val colors = MaterialTheme.colorScheme
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(18.dp).testTag("componentPreview"), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        Text("Podgląd komponentów", style = MaterialTheme.typography.headlineSmall)
        Text("DEV · dane demonstracyjne", color = colors.onSurfaceVariant, style = MaterialTheme.typography.bodySmall)
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            listOf(ThemeMode.System to "System", ThemeMode.Light to "Jasny", ThemeMode.Dark to "Ciemny").forEach { (item, label) ->
                WindButton(label, { onMode(item) }, Modifier.weight(1f).testTag("theme-${item.name}").semantics { selected = item == mode }, kind = if (item == mode) ButtonKind.Primary else ButtonKind.Secondary)
            }
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            WindSwitch("Kolor z tapety", dynamic, onDynamic, Modifier.testTag("dynamicColor"))
        } else Text("Android 10/11 · paleta WindConnect", color = colors.onSurfaceVariant, style = MaterialTheme.typography.bodySmall)
        Text(if (dynamic && Build.VERSION.SDK_INT >= 31) "Material You · kolor systemowy" else "Paleta z makiet WN-20", modifier = Modifier.testTag("paletteSource"), style = MaterialTheme.typography.bodySmall)
        WindCard(Modifier.fillMaxWidth()) {
            Text("SZYMON-PC", style = MaterialTheme.typography.headlineSmall)
            Text("Przykład · Połączono · Wi-Fi", color = colors.onSurfaceVariant, style = MaterialTheme.typography.bodySmall)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                WindAction("Wyślij plik", "Do komputera", "send", { feedback = "Przykład: wyślij plik" }, Modifier.weight(1f).testTag("actionSend"))
                WindAction("Schowek", "Wyślij tekst", "clipboard", { feedback = "Przykład: schowek" }, Modifier.weight(1f))
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                WindAction("Touchpad", "Steruj kursorem", "touch", { feedback = "Przykład: touchpad" }, Modifier.weight(1f))
                WindAction("Zablokuj PC", "Zdalna blokada", "lock", { feedback = "Przykład: blokada" }, Modifier.weight(1f))
            }
        }
        WindCard(Modifier.fillMaxWidth()) {
            Text("Przykładowy transfer", style = MaterialTheme.typography.titleMedium)
            Text(if (complete) "Wysłano · 240 MB" else "23 zdjęcia · 154 z 240 MB", style = MaterialTheme.typography.bodyMedium)
            Box(Modifier.fillMaxWidth().height(5.dp).background(colors.surface).semantics { progressBarRangeInfo = ProgressBarRangeInfo(if (complete) 1f else .64f, 0f..1f) }) {
                Box(Modifier.fillMaxWidth(if (complete) 1f else .64f).fillMaxHeight().background(colors.primary))
            }
            WindButton(if (complete) "Resetuj przykład" else "Zakończ przykład", { complete = !complete }, kind = ButtonKind.Secondary)
        }
        WindCard(Modifier.fillMaxWidth()) {
            Text("Pola i przełączniki", style = MaterialTheme.typography.titleMedium)
            WindSwitch("Automatyczny schowek", clipboard, { clipboard = it }, Modifier.testTag("sampleSwitch"))
            WindField(name, { name = it }, "Nazwa komputera", Modifier.testTag("sampleName"), errorText = if (validated && name.isBlank()) "Wpisz nazwę komputera." else null)
            WindButton("Sprawdź nazwę", { validated = true; if (name.isNotBlank()) feedback = "Przykład: nazwa poprawna" }, Modifier.testTag("validateName"))
            WindButton("Niedostępny", {}, enabled = false, kind = ButtonKind.Secondary)
        }
        Text("Zażółć gęślą jaźń. 0123456789 · DM Sans", style = MaterialTheme.typography.bodyMedium)
        Text(feedback, modifier = Modifier.testTag("previewFeedback").semantics { liveRegion = LiveRegionMode.Polite }, color = colors.onSurfaceVariant, style = MaterialTheme.typography.bodySmall)
        Text("Podgląd nie łączy z komputerem i nie wysyła plików.", color = colors.onSurfaceVariant, style = MaterialTheme.typography.bodySmall)
    }
}
