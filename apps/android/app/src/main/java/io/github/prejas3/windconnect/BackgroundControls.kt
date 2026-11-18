package io.github.prejas3.windconnect

import io.github.prejas3.windconnect.ui.*
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.heading

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
    // WN-130: the start/stop button became the row's switch; warnings (notifications off, battery) stay visible with their action.
    WindSettingRow("bolt", str(R.string.background_title), str(R.string.sum_background), checked = enabled, enabled = hasPairedDevice, switchTag = "backgroundToggle",
        onCheckedChange = { on ->
            if (!on) ConnectionService.stop(context) else {
                ConnectionService.start(context, enable = true)
                if (Build.VERSION.SDK_INT >= 33 && !notifications) permission.launch(Manifest.permission.POST_NOTIFICATIONS)
            }
            refresh()
        },
        status = message.ifEmpty { if (enabled) str(R.string.background_on_state) else str(R.string.off) }, statusTag = "backgroundStatus",
        warning = message.isNotEmpty() || (enabled && (!notifications || !battery)),
        actions = if (!enabled) null else { {
            if (!notifications) {
                Text(str(R.string.background_notifications_off), color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
                WindButton(str(R.string.notification_settings), {
                    context.startActivity(Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName))
                }, kind = ButtonKind.Secondary)
            }
            if (!battery) {
                Text(str(R.string.battery_optimized), style = MaterialTheme.typography.bodySmall)
                WindButton(str(R.string.battery_optimization), {
                    context.startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))
                }, modifier = Modifier.testTag("batterySettings"), kind = ButtonKind.Secondary)
            }
        } },
        more = if (!enabled) null else { { Text(if (battery) str(R.string.battery_unrestricted) else str(R.string.battery_optimized)); Text(str(R.string.battery_samsung)) } })
}
