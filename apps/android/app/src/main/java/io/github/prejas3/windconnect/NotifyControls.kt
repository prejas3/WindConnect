package io.github.prejas3.windconnect

import io.github.prejas3.windconnect.ui.*
import androidx.compose.ui.semantics.heading

import android.content.ComponentName
import android.content.Intent
import android.os.Build
import android.provider.Settings
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.prejas3.windconnect.core.NotifyAppMode
import io.github.prejas3.windconnect.core.NotifyAppsState
import io.github.prejas3.windconnect.core.NotifyListenerService
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import io.github.prejas3.windconnect.ui.WindField
import io.github.prejas3.windconnect.ui.ButtonKind
import io.github.prejas3.windconnect.ui.WindButton
import io.github.prejas3.windconnect.ui.WindCard
import io.github.prejas3.windconnect.ui.WindSwitch
import io.github.prejas3.windconnect.ui.WindSegmented

/**
 * WN-42 Settings: the phone's notifications on the PC. Its own consent (PROTOCOL.md §9.6 rule 6): a separate system access entry,
 * "WindConnect: powiadomienia na komputerze", and this switch, off by default. Says plainly what is read and where it goes.
 */
@Composable
fun NotifyControls() {
    val context = LocalContext.current
    val connections = (context.applicationContext as WindApplication).connections
    val allowed by connections.notifyAllowed.collectAsStateWithLifecycle()
    val access by connections.notifyAccess.collectAsStateWithLifecycle()
    val lifecycle = LocalLifecycleOwner.current
    DisposableEffect(lifecycle) {
        connections.refreshNotify()
        val observer = object : DefaultLifecycleObserver { override fun onResume(owner: LifecycleOwner) = connections.refreshNotify() }
        lifecycle.lifecycle.addObserver(observer)
        onDispose { lifecycle.lifecycle.removeObserver(observer) }
    }
    val openAccess = {
        val intent = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            Intent(Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS).putExtra(
                Settings.EXTRA_NOTIFICATION_LISTENER_COMPONENT_NAME,
                ComponentName(context, NotifyListenerService::class.java).flattenToString())
        } else Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)
        context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }
    WindSettingRow("bell", str(R.string.notify_title), str(R.string.sum_notify), checked = allowed, onCheckedChange = connections::setNotifyAllowed,
        switchTag = "notifyAllowed", status = if (access) null else str(R.string.needs_access), statusTag = "notifyAccess", warning = allowed && !access,
        actions = {
            if (!access) WindButton(str(R.string.access_grant_short), openAccess, Modifier.testTag("notifyAccessSettings"), kind = ButtonKind.Secondary)
            if (allowed) NotifyApps(connections)
        },
        more = {
            Text(str(R.string.notify_about)); Text(str(R.string.notify_access_about))
            if (access) WindButton(str(R.string.access_change), openAccess, kind = ButtonKind.Quiet)
        })
}

/**
 * WN-45: which apps go to the PC. A blocked app's notifications never leave the phone. The list: apps seen posting something that
 * could go to the PC (no QUERY_ALL_PACKAGES), by name; a search box once there are more than 12.
 */
@Composable
private fun NotifyApps(connections: ConnectionOwner) {
    val apps by connections.notifyApps.collectAsStateWithLifecycle()
    val state = apps ?: return
    NotifyAppsList(state, connections::appLabel, connections::setNotifyAppMode, connections::setNotifyApp)
}

/** The list itself, stateless (instrumentation-tested). */
@Composable
internal fun NotifyAppsList(state: NotifyAppsState, label: (String) -> String, onMode: (NotifyAppMode) -> Unit, onApp: (String, Boolean) -> Unit) {
    var query by rememberSaveable { mutableStateOf("") }
    Text(str(R.string.notify_apps), style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(top = 8.dp))
    val modes = listOf(NotifyAppMode.AllExcept to str(R.string.notify_all_but_off), NotifyAppMode.OnlyChosen to str(R.string.notify_only_chosen))
    WindSegmented(modes.map { it.first }, state.mode, { mode -> modes.first { it.first == mode }.second }, onMode,
        Modifier.testTag("notifyAppMode"), tag = { "notifyAppMode$it" })
    Text(when {
        state.sendsNothing -> str(R.string.notify_none_sent)
        state.mode == NotifyAppMode.OnlyChosen -> str(R.string.notify_only_on)
        else -> str(R.string.notify_all_on)
    }, style = MaterialTheme.typography.bodySmall, modifier = Modifier.testTag("notifyAppsState").semantics { liveRegion = LiveRegionMode.Polite })
    val labelled = remember(state.seen) { state.seen.map { it to label(it) }.sortedBy { it.second.lowercase() } }
    if (labelled.isEmpty()) {
        Text(str(R.string.notify_apps_later), style = MaterialTheme.typography.bodySmall)
        return
    }
    if (labelled.size > 12) WindField(query, { query = it }, str(R.string.notify_search), Modifier.testTag("notifyAppSearch"))
    labelled.filter { query.isBlank() || it.second.contains(query.trim(), ignoreCase = true) }.forEach { (pkg, label) ->
        WindSwitch(label, state.allows(pkg), { on -> onApp(pkg, on) }, Modifier.testTag("notifyApp:$pkg"))
    }
}
