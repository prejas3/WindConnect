package io.github.prejas3.windconnect

import android.os.SystemClock
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.prejas3.windconnect.core.PcVolume
import io.github.prejas3.windconnect.ui.ButtonKind
import io.github.prejas3.windconnect.ui.WindButton
import io.github.prejas3.windconnect.ui.WindCard
import kotlinx.coroutines.delay
import kotlin.math.roundToInt

/** WN-48: the PC's output volume, shown while the PC shares it (PROTOCOL.md §9.9). */
@Composable
fun PcVolumeCard() {
    val connections = (LocalContext.current.applicationContext as WindApplication).connections
    val volume by connections.pcVolume.collectAsStateWithLifecycle()
    val shown = volume ?: return
    WindCard(Modifier.fillMaxWidth().testTag("pcVolume")) {
        PcVolumeControls(shown, { connections.setPcVolume(level = it) }, { connections.setPcVolume(muted = it) })
    }
}

/** How long after a local change a pushed level (often the echo of our own `Set`) doesn't move the slider. */
const val VOLUME_HOLD_MS = 500L

/**
 * The slider and mute, stateless towards the PC (instrumentation-tested). RULE:no bounce-back: while dragging, and for
 * [VOLUME_HOLD_MS] after the last local change, a pushed level doesn't move it; then the latest pushed level wins.
 */
@Composable
internal fun PcVolumeControls(volume: PcVolume, onLevel: (Int) -> Unit, onMute: (Boolean) -> Unit) {
    var level by remember { mutableStateOf(volume.level) }
    var muted by remember { mutableStateOf(volume.muted) }
    var dragging by remember { mutableStateOf(false) }
    var localAt by remember { mutableLongStateOf(0L) }
    LaunchedEffect(volume, dragging, localAt) {
        if (dragging) return@LaunchedEffect
        val wait = VOLUME_HOLD_MS - (SystemClock.uptimeMillis() - localAt)
        if (wait > 0) delay(wait)
        level = volume.level
        muted = volume.muted
    }
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
        Text(str(R.string.pc_volume), style = MaterialTheme.typography.titleSmall)
        Text(if (muted) str(R.string.muted) else "$level%", style = MaterialTheme.typography.bodySmall, modifier = Modifier.testTag("pcVolumeValue"))
    }
    Slider(
        value = level / 100f,
        onValueChange = { v ->
            dragging = true
            localAt = SystemClock.uptimeMillis()
            level = (v * 100).roundToInt()
            onLevel(level)
        },
        onValueChangeFinished = { dragging = false; localAt = SystemClock.uptimeMillis() },
        steps = 19,
        modifier = Modifier.fillMaxWidth().testTag("pcVolumeSlider").semantics {
            contentDescription = str(R.string.pc_volume)
            stateDescription = if (muted) str(R.string.volume_muted_state, "$level%") else "$level%"
        },
    )
    WindButton(if (muted) str(R.string.pc_unmute) else str(R.string.pc_mute), {
        localAt = SystemClock.uptimeMillis()
        muted = !muted
        onMute(muted)
    }, Modifier.testTag("pcVolumeMute"), kind = ButtonKind.Quiet)
}
