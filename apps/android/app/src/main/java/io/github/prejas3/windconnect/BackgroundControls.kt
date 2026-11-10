package io.github.prejas3.windconnect

import android.Manifest
import android.app.NotificationManager
import android.content.Intent
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.compose.LocalLifecycleOwner
import io.github.prejas3.windconnect.ui.ButtonKind
import io.github.prejas3.windconnect.ui.WindButton
import io.github.prejas3.windconnect.ui.WindCard

@Composable
fun BackgroundControls(hasPairedDevice: Boolean) {
    val context = LocalContext.current
    var enabled by remember { mutableStateOf(ConnectionService.enabled(context)) }
    var battery by remember { mutableStateOf(false) }
    var notifications by remember { mutableStateOf(true) }
    var message by remember { mutableStateOf("") }
    fun refresh() {
        enabled = ConnectionService.enabled(context)
        battery = context.getSystemService(PowerManager::class.java).isIgnoringBatteryOptimizations(context.packageName)
        notifications = context.getSystemService(NotificationManager::class.java).areNotificationsEnabled()
        message = ConnectionService.preferences(context).getString("error", "").orEmpty()
    }
    val lifecycle = LocalLifecycleOwner.current
    DisposableEffect(lifecycle) {
        refresh()
        val preferences = ConnectionService.preferences(context)
        val changes = android.content.SharedPreferences.OnSharedPreferenceChangeListener { _, _ -> refresh() }
        preferences.registerOnSharedPreferenceChangeListener(changes)
        val observer = object : DefaultLifecycleObserver { override fun onResume(owner: LifecycleOwner) = refresh() }
        lifecycle.lifecycle.addObserver(observer)
        onDispose { lifecycle.lifecycle.removeObserver(observer); preferences.unregisterOnSharedPreferenceChangeListener(changes) }
    }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { refresh() }
    LaunchedEffect(enabled) {
        val preferences = ConnectionService.preferences(context)
        if (enabled && Build.VERSION.SDK_INT >= 33 && !notifications && !preferences.getBoolean("permissionAsked", false)) {
            preferences.edit().putBoolean("permissionAsked", true).apply()
            permission.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }
    WindCard {
        Text("Połączenie w tle", style = MaterialTheme.typography.titleMedium)
        Text(if (enabled) "Włączone · stan komputera i szybkie akcje w powiadomieniu" else "Wyłączone", modifier = Modifier.testTag("backgroundStatus"))
        WindButton(if (enabled) "Zatrzymaj połączenie w tle" else "Włącz połączenie w tle", {
            if (enabled) ConnectionService.stop(context) else {
                ConnectionService.start(context, enable = true)
                if (Build.VERSION.SDK_INT >= 33 && !notifications) permission.launch(Manifest.permission.POST_NOTIFICATIONS)
            }
            refresh()
        }, enabled = hasPairedDevice, modifier = Modifier.testTag("backgroundToggle"), kind = ButtonKind.Secondary)
        if (enabled && !notifications) {
            Text("Powiadomienia są wyłączone. Włącz je, aby widzieć urządzenie i szybkie akcje.")
            WindButton("Ustawienia powiadomień", {
                context.startActivity(Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName))
            }, kind = ButtonKind.Quiet)
        }
        if (enabled) {
            Text(if (battery) "Oszczędzanie baterii: aplikacja może utrzymywać połączenie przy wyłączonym ekranie." else "Aby utrzymać połączenie przez noc, wyłącz optymalizację baterii dla WindConnect. Utrzymywanie aktywnego połączenia zwiększa zużycie baterii.")
            if (!battery) WindButton("Optymalizacja baterii", {
                context.startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))
            }, modifier = Modifier.testTag("batterySettings"), kind = ButtonKind.Quiet)
            Text("Samsung One UI: dodaj WindConnect do aplikacji, które nigdy nie są usypiane, w Ustawienia → Bateria → Limity użycia w tle.", style = MaterialTheme.typography.bodySmall)
        }
        if (message.isNotEmpty()) Text(message)
    }
}
