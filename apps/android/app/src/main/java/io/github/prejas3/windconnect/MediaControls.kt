package io.github.prejas3.windconnect

import io.github.prejas3.windconnect.ui.*
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.heading

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
    // WN-130: access missing is the one thing shown in red with its action; the rest is under "More".
    val openAccess = { context.startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
    WindSettingRow("music", str(R.string.media_title), str(R.string.sum_media), checked = allowed, onCheckedChange = connections::setMediaAllowed,
        switchTag = "mediaAllowed", status = if (access) null else str(R.string.needs_access), statusTag = "mediaAccess", warning = allowed && !access,
        actions = if (access) null else { { WindButton(str(R.string.access_grant_short), openAccess, Modifier.testTag("mediaAccessSettings"), kind = ButtonKind.Secondary) } },
        more = {
            Text(str(R.string.media_about)); Text(str(R.string.media_access_about))
            if (access) WindButton(str(R.string.access_change), openAccess, kind = ButtonKind.Quiet)
        })
}

/** WN-47 / WN-48, the other direction: the PC's player and volume on the phone (settings group "Computer on the phone"). */
@Composable
fun PcMediaControls() {
    val connections = (LocalContext.current.applicationContext as WindApplication).connections
    val pcAllowed by connections.pcMediaAllowed.collectAsStateWithLifecycle()
    WindSettingRow("music", str(R.string.media_pc_player), str(R.string.sum_pc_player), checked = pcAllowed, onCheckedChange = connections::setPcMediaAllowed,
        switchTag = "pcMediaAllowed", more = { Text(str(R.string.media_pc_player_about)) })
    WindDivider()
    val volumeAllowed by connections.volumeAllowed.collectAsStateWithLifecycle()
    WindSettingRow("volume", str(R.string.media_volume), str(R.string.sum_volume), checked = volumeAllowed, onCheckedChange = connections::setVolumeAllowed,
        switchTag = "volumeAllowed", more = { Text(str(R.string.media_volume_about)) })
}
