package io.github.prejas3.windconnect.ui

import io.github.prejas3.windconnect.R
import androidx.compose.ui.res.stringResource
import android.os.Build
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.draw.clip
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
fun WindApp(forceContent: Boolean = false, onSettings: (() -> Unit)? = null, connection: @Composable () -> Unit) {
    val preferences = LocalContext.current.getSharedPreferences("appearance", 0)
    var modeName by rememberSaveable { mutableStateOf(preferences.getString("theme", ThemeMode.System.name) ?: ThemeMode.System.name) }
    var dynamic by rememberSaveable { mutableStateOf(preferences.getBoolean("dynamic", true)) }
    var preview by rememberSaveable { mutableStateOf(false) }
    LaunchedEffect(forceContent) { if (forceContent) preview = false }
    val mode = ThemeMode.entries.firstOrNull { it.name == modeName } ?: ThemeMode.System
    WindConnectTheme(mode, dynamic) {
        Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).safeDrawingPadding().imePadding()) {
            val settingsLabel = stringResource(R.string.settings)
            Row(Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 8.dp), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                Text(stringResource(R.string.app_name), modifier = Modifier.weight(1f).padding(end = 12.dp, top = 10.dp, bottom = 10.dp), style = MaterialTheme.typography.titleLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (BuildConfig.DEBUG && !forceContent) WindButton(if (preview) stringResource(R.string.preview_connection) else stringResource(R.string.preview_toggle), onClick = { preview = !preview }, modifier = Modifier.testTag("previewToggle"), kind = ButtonKind.Quiet)
                if (!forceContent && onSettings != null) Box(Modifier.size(48.dp).clip(RoundedCornerShape(12.dp)).clickable(role = Role.Button, onClick = onSettings)
                    .semantics { contentDescription = settingsLabel }.testTag("openSettings"), contentAlignment = androidx.compose.ui.Alignment.Center) { WindIcon("gear") }
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
    var feedback by rememberSaveable { mutableStateOf(R.string.preview_all_sample) }
    var complete by rememberSaveable { mutableStateOf(false) }
    val colors = MaterialTheme.colorScheme
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(18.dp).testTag("componentPreview"), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        Text(stringResource(R.string.preview_title), style = MaterialTheme.typography.headlineSmall, modifier = Modifier.semantics { heading() })
        Text(stringResource(R.string.preview_dev), color = colors.onSurfaceVariant, style = MaterialTheme.typography.bodySmall)
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            listOf(ThemeMode.System to stringResource(R.string.theme_system), ThemeMode.Light to stringResource(R.string.theme_light), ThemeMode.Dark to stringResource(R.string.theme_dark)).forEach { (item, label) ->
                WindButton(label, { onMode(item) }, Modifier.weight(1f).testTag("theme-${item.name}").semantics { selected = item == mode }, kind = if (item == mode) ButtonKind.Primary else ButtonKind.Secondary)
            }
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            WindSwitch(stringResource(R.string.preview_wallpaper_color), dynamic, onDynamic, Modifier.testTag("dynamicColor"))
        } else Text(stringResource(R.string.preview_palette), color = colors.onSurfaceVariant, style = MaterialTheme.typography.bodySmall)
        Text(if (dynamic && Build.VERSION.SDK_INT >= 31) stringResource(R.string.preview_material_you) else stringResource(R.string.preview_mock_palette), modifier = Modifier.testTag("paletteSource"), style = MaterialTheme.typography.bodySmall)
        WindCard(Modifier.fillMaxWidth()) {
            Text("SZYMON-PC", style = MaterialTheme.typography.headlineSmall, modifier = Modifier.semantics { heading() })
            Text(stringResource(R.string.preview_pc_state), color = colors.onSurfaceVariant, style = MaterialTheme.typography.bodySmall)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                WindAction(stringResource(R.string.action_send_file), stringResource(R.string.preview_to_computer), "send", { feedback = R.string.preview_sample_send }, Modifier.weight(1f).testTag("actionSend"))
                WindAction(stringResource(R.string.action_clipboard), stringResource(R.string.preview_send_text), "clipboard", { feedback = R.string.preview_sample_clipboard }, Modifier.weight(1f))
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                WindAction(stringResource(R.string.action_touchpad), stringResource(R.string.preview_cursor), "touch", { feedback = R.string.preview_sample_touchpad }, Modifier.weight(1f))
                WindAction(stringResource(R.string.action_lock_pc), stringResource(R.string.preview_remote_lock), "lock", { feedback = R.string.preview_sample_lock }, Modifier.weight(1f))
            }
        }
        WindCard(Modifier.fillMaxWidth()) {
            Text(stringResource(R.string.preview_transfer), style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
            Text(if (complete) stringResource(R.string.preview_sent) else stringResource(R.string.preview_progress), style = MaterialTheme.typography.bodyMedium)
            Box(Modifier.fillMaxWidth().height(5.dp).background(colors.surface).semantics { progressBarRangeInfo = ProgressBarRangeInfo(if (complete) 1f else .64f, 0f..1f) }) {
                Box(Modifier.fillMaxWidth(if (complete) 1f else .64f).fillMaxHeight().background(colors.primary))
            }
            WindButton(if (complete) stringResource(R.string.preview_reset) else stringResource(R.string.preview_finish), { complete = !complete }, kind = ButtonKind.Secondary)
        }
        WindCard(Modifier.fillMaxWidth()) {
            Text(stringResource(R.string.preview_fields), style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
            WindSwitch(stringResource(R.string.preview_auto_clipboard), clipboard, { clipboard = it }, Modifier.testTag("sampleSwitch"))
            WindField(name, { name = it }, stringResource(R.string.preview_pc_name), Modifier.testTag("sampleName"), errorText = if (validated && name.isBlank()) stringResource(R.string.preview_name_required) else null)
            WindButton(stringResource(R.string.preview_check_name), { validated = true; if (name.isNotBlank()) feedback = R.string.preview_sample_name }, Modifier.testTag("validateName"))
            WindButton(stringResource(R.string.unavailable_m), {}, enabled = false, kind = ButtonKind.Secondary)
        }
        Text(stringResource(R.string.preview_font), style = MaterialTheme.typography.bodyMedium)
        Text(stringResource(feedback), modifier = Modifier.testTag("previewFeedback").semantics { liveRegion = LiveRegionMode.Polite }, color = colors.onSurfaceVariant, style = MaterialTheme.typography.bodySmall)
        Text(stringResource(R.string.preview_note), color = colors.onSurfaceVariant, style = MaterialTheme.typography.bodySmall)
    }
}
