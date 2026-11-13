package io.github.prejas3.windconnect

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
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
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
    WindCard {
        Text("Powiadomienia na komputerze", style = MaterialTheme.typography.titleMedium)
        Text("Nowe powiadomienia z telefonu (aplikacja, tytuł, treść, ikona) trafiają na sparowany komputer, gdy jest połączony. " +
            "Nic nie idzie przez chmurę ani nie jest zapisywane. Stałe powiadomienia (np. odtwarzacz, nawigacja w tle) są pomijane. " +
            "Z komputera można odpowiedzieć na wiadomość i odrzucić powiadomienie; odrzucone na jednym urządzeniu znika z drugiego.")
        Text("To osobna zgoda: w ustawieniach Androida wybierz „WindConnect: powiadomienia na komputerze”. " +
            "Dostęp dla multimediów nie wystarcza i nie jest tu używany. Android 15 może ukrywać treść wrażliwą (np. kody SMS); " +
            "komputer zobaczy wtedy „Treść ukryta” albo nic.", style = MaterialTheme.typography.bodySmall)
        Text(if (access) "Dostęp: przyznany" else "Dostęp: brak", modifier = Modifier.testTag("notifyAccess"))
        WindButton(if (access) "Zmień dostęp w ustawieniach" else "Przyznaj dostęp w ustawieniach", {
            val intent = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                Intent(Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS).putExtra(
                    Settings.EXTRA_NOTIFICATION_LISTENER_COMPONENT_NAME,
                    ComponentName(context, NotifyListenerService::class.java).flattenToString())
            } else Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)
            context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }, modifier = Modifier.testTag("notifyAccessSettings"), kind = ButtonKind.Secondary)
        WindSwitch("Powiadomienia na komputerze", allowed, connections::setNotifyAllowed, Modifier.testTag("notifyAllowed"))
        if (allowed) NotifyApps(connections)
    }
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
    Text("Aplikacje", style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(top = 8.dp))
    val modes = listOf(NotifyAppMode.AllExcept to "Wszystkie oprócz wyłączonych", NotifyAppMode.OnlyChosen to "Tylko wybrane")
    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().testTag("notifyAppMode")) {
        modes.forEachIndexed { index, (mode, label) ->
            SegmentedButton(selected = state.mode == mode, onClick = { onMode(mode) },
                shape = SegmentedButtonDefaults.itemShape(index, modes.size), modifier = Modifier.testTag("notifyAppMode$mode")) {
                Text(label, style = MaterialTheme.typography.labelMedium)
            }
        }
    }
    Text(when {
        state.sendsNothing -> "Nie wysyłasz żadnych powiadomień. Wybierz aplikacje poniżej."
        state.mode == NotifyAppMode.OnlyChosen -> "Na komputer trafiają tylko włączone aplikacje; nowe są wyłączone."
        else -> "Na komputer trafiają wszystkie aplikacje poza wyłączonymi; nowe są włączone."
    }, style = MaterialTheme.typography.bodySmall, modifier = Modifier.testTag("notifyAppsState").semantics { liveRegion = LiveRegionMode.Polite })
    val labelled = remember(state.seen) { state.seen.map { it to label(it) }.sortedBy { it.second.lowercase() } }
    if (labelled.isEmpty()) {
        Text("Aplikacje pojawią się tu, gdy wyślą pierwsze powiadomienie.", style = MaterialTheme.typography.bodySmall)
        return
    }
    if (labelled.size > 12) WindField(query, { query = it }, "Szukaj aplikacji", Modifier.testTag("notifyAppSearch"))
    labelled.filter { query.isBlank() || it.second.contains(query.trim(), ignoreCase = true) }.forEach { (pkg, label) ->
        WindSwitch(label, state.allows(pkg), { on -> onApp(pkg, on) }, Modifier.testTag("notifyApp:$pkg"))
    }
}
