package io.github.prejas3.windconnect

import android.os.SystemClock
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.prejas3.windconnect.ui.ButtonKind
import io.github.prejas3.windconnect.ui.WindButton
import io.github.prejas3.windconnect.ui.WindCard
import io.github.prejas3.windconnect.ui.*
import androidx.compose.ui.semantics.heading
import kotlinx.coroutines.delay

private fun clock(ms: Long): String { val s = (ms / 1000).coerceAtLeast(0); return "%d:%02d".format(s / 60, s % 60) }

/**
 * WN-47: what plays on the PC, with its controls (PROTOCOL.md §9.6). Texts were cleaned in [io.github.prejas3.windconnect.core.PcPlayer]
 * and are shown as plain Text; progress is extrapolated locally, nothing is polled.
 */
@Composable
fun PcMediaCard() {
    // WN-130: player and volume in one "On the computer" card; each part only while the PC shares it.
    val connections = (LocalContext.current.applicationContext as WindApplication).connections
    val media by connections.pcMedia.collectAsStateWithLifecycle()
    val volume by connections.pcVolume.collectAsStateWithLifecycle()
    if (media == null && volume == null) return
    WindCard(Modifier.fillMaxWidth().testTag("pcCard")) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(str(R.string.on_the_pc), style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f).semantics { heading() })
            media?.let { Text(it.app, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis) }
        }
        media?.let { PcMediaPart(it, connections) }
        volume?.let { shown ->
            if (media != null) WindDivider()
            Column(Modifier.testTag("pcVolume"), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                PcVolumeControls(shown, { connections.setPcVolume(level = it) }, { connections.setPcVolume(muted = it) })
            }
        }
    }
}

@Composable
private fun PcMediaPart(shown: io.github.prejas3.windconnect.core.PcMedia, connections: ConnectionOwner) {
    var now by remember { mutableLongStateOf(SystemClock.elapsedRealtime()) }
    LaunchedEffect(shown) { while (shown.playing) { now = SystemClock.elapsedRealtime(); delay(1_000) } }
    val position = shown.positionAt(now)
    val can = { bit: Long -> shown.actions and bit != 0L }
    Column(Modifier.fillMaxWidth().testTag("pcMedia"), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            shown.art?.let { Image(it.asImageBitmap(), contentDescription = null, modifier = Modifier.size(56.dp).clip(WindShapes.small)) }
            Column(Modifier.weight(1f)) {
                Text(shown.title, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.testTag("pcMediaTitle"))
                listOfNotNull(shown.artist, shown.album).joinToString(" · ").takeIf { it.isNotEmpty() }?.let {
                    Text(it, style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
        }
        shown.durationMs?.takeIf { it > 0 }?.let { total ->
            var dragged by remember { mutableStateOf<Float?>(null) }
            WindSlider(value = dragged ?: (position.toFloat() / total), enabled = can(16),
                onValueChange = { dragged = it }, valueLabel = { clock((it * total).toLong()) },
                onValueChangeFinished = { dragged?.let { connections.pcMediaCommand("seek", (it * total).toLong()) }; dragged = null },
                modifier = Modifier.fillMaxWidth().semantics { contentDescription = str(R.string.media_position, clock(position), clock(total)) })
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                Text(clock(position), style = MaterialTheme.typography.bodySmall); Text(clock(total), style = MaterialTheme.typography.bodySmall)
            }
        }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(24.dp, Alignment.CenterHorizontally), verticalAlignment = Alignment.CenterVertically) {
            WindIconButton("previous", str(R.string.media_previous), { connections.pcMediaCommand("previous") }, Modifier.testTag("pcMediaPrevious"), enabled = can(8))
            WindIconButton(if (shown.playing) "pause" else "play", if (shown.playing) str(R.string.media_pause) else str(R.string.media_play), { connections.pcMediaCommand(if (shown.playing) "pause" else "play") },
                Modifier.testTag("pcMediaToggle"), enabled = can(if (shown.playing) 2 else 1), primary = true)
            WindIconButton("next", str(R.string.media_next), { connections.pcMediaCommand("next") }, Modifier.testTag("pcMediaNext"), enabled = can(4))
        }
    }
}
