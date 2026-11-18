package io.github.prejas3.windconnect

import android.os.SystemClock
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
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
import kotlinx.coroutines.delay

private fun clock(ms: Long): String { val s = (ms / 1000).coerceAtLeast(0); return "%d:%02d".format(s / 60, s % 60) }

/**
 * WN-47: what plays on the PC, with its controls (PROTOCOL.md §9.6). Texts were cleaned in [io.github.prejas3.windconnect.core.PcPlayer]
 * and are shown as plain Text; progress is extrapolated locally, nothing is polled.
 */
@Composable
fun PcMediaCard() {
    val connections = (LocalContext.current.applicationContext as WindApplication).connections
    val media by connections.pcMedia.collectAsStateWithLifecycle()
    val shown = media ?: return
    var now by remember { mutableLongStateOf(SystemClock.elapsedRealtime()) }
    LaunchedEffect(shown) { while (shown.playing) { now = SystemClock.elapsedRealtime(); delay(1_000) } }
    val position = shown.positionAt(now)
    val can = { bit: Long -> shown.actions and bit != 0L }
    WindCard(Modifier.fillMaxWidth().testTag("pcMedia")) {
        Text(str(R.string.pc_media_app, shown.app), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            shown.art?.let { Image(it.asImageBitmap(), contentDescription = null, modifier = Modifier.size(56.dp).clip(RoundedCornerShape(8.dp))) }
            Column(Modifier.weight(1f)) {
                Text(shown.title, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.testTag("pcMediaTitle"))
                listOfNotNull(shown.artist, shown.album).joinToString(" · ").takeIf { it.isNotEmpty() }?.let {
                    Text(it, style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
        }
        shown.durationMs?.takeIf { it > 0 }?.let { total ->
            var dragged by remember { mutableStateOf<Float?>(null) }
            Slider(value = dragged ?: (position.toFloat() / total), enabled = can(16),
                onValueChange = { dragged = it },
                onValueChangeFinished = { dragged?.let { connections.pcMediaCommand("seek", (it * total).toLong()) }; dragged = null },
                modifier = Modifier.fillMaxWidth().semantics { contentDescription = str(R.string.media_position, clock(position), clock(total)) })
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                Text(clock(position), style = MaterialTheme.typography.bodySmall); Text(clock(total), style = MaterialTheme.typography.bodySmall)
            }
        }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally)) {
            WindButton(str(R.string.media_previous), { connections.pcMediaCommand("previous") }, Modifier.testTag("pcMediaPrevious"), enabled = can(8), kind = ButtonKind.Quiet)
            WindButton(if (shown.playing) str(R.string.media_pause) else str(R.string.media_play), { connections.pcMediaCommand(if (shown.playing) "pause" else "play") },
                Modifier.testTag("pcMediaToggle"), enabled = can(if (shown.playing) 2 else 1))
            WindButton(str(R.string.media_next), { connections.pcMediaCommand("next") }, Modifier.testTag("pcMediaNext"), enabled = can(4), kind = ButtonKind.Quiet)
        }
    }
}
