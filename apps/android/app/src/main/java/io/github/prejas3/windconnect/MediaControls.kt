package io.github.prejas3.windconnect

import android.content.Intent
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
import io.github.prejas3.windconnect.ui.ButtonKind
import io.github.prejas3.windconnect.ui.WindButton
import io.github.prejas3.windconnect.ui.WindCard
import io.github.prejas3.windconnect.ui.WindSwitch

/**
 * WN-46 Settings: the phone's player on the PC. Android's word for the access is "notification access"; the card says plainly
 * that WindConnect uses it only for the player (PROTOCOL.md §9.6 rule 6: WN-42 needs its own consent).
 */
@Composable
fun MediaControls() {
    val context = LocalContext.current
    val connections = (context.applicationContext as WindApplication).connections
    val allowed by connections.mediaAllowed.collectAsStateWithLifecycle()
    val access by connections.mediaAccess.collectAsStateWithLifecycle()
    val lifecycle = LocalLifecycleOwner.current
    DisposableEffect(lifecycle) {
        connections.refreshMedia()
        val observer = object : DefaultLifecycleObserver { override fun onResume(owner: LifecycleOwner) = connections.refreshMedia() }
        lifecycle.lifecycle.addObserver(observer)
        onDispose { lifecycle.lifecycle.removeObserver(observer) }
    }
    WindCard {
        Text("Multimedia na komputerze", style = MaterialTheme.typography.titleMedium)
        Text("Komputer pokazuje, co gra na telefonie (tytuł, okładkę, postęp), i może sterować odtwarzaniem.")
        Text("Android nazywa to dostępem do powiadomień. WindConnect używa go tylko do odczytu odtwarzacza; nie czyta treści powiadomień.",
            style = MaterialTheme.typography.bodySmall)
        Text(if (access) "Dostęp: przyznany" else "Dostęp: brak", modifier = Modifier.testTag("mediaAccess"))
        WindButton(if (access) "Zmień dostęp w ustawieniach" else "Przyznaj dostęp w ustawieniach", {
            context.startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }, modifier = Modifier.testTag("mediaAccessSettings"), kind = ButtonKind.Secondary)
        WindSwitch("Pokaż multimedia na komputerze", allowed, connections::setMediaAllowed, Modifier.testTag("mediaAllowed"))
        // WN-47: the other direction.
        val pcAllowed by connections.pcMediaAllowed.collectAsStateWithLifecycle()
        WindSwitch("Pokaż odtwarzacz komputera", pcAllowed, connections::setPcMediaAllowed, Modifier.testTag("pcMediaAllowed"))
        Text("Co gra na komputerze, w aplikacji i w sterowaniu multimediami Androida. Tytuły widać też na ekranie blokady, jak przy innych odtwarzaczach.",
            style = MaterialTheme.typography.bodySmall)
        // WN-48: volume both ways.
        val volumeAllowed by connections.volumeAllowed.collectAsStateWithLifecycle()
        WindSwitch("Głośność z komputera", volumeAllowed, connections::setVolumeAllowed, Modifier.testTag("volumeAllowed"))
        Text("Komputer pokazuje i zmienia głośność multimediów telefonu, a aplikacja pokazuje i zmienia głośność komputera.",
            style = MaterialTheme.typography.bodySmall)
    }
}
