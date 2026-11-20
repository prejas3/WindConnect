package io.github.prejas3.windconnect

import io.github.prejas3.windconnect.ui.*
import androidx.compose.ui.semantics.heading

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.prejas3.windconnect.ui.WindButton
import io.github.prejas3.windconnect.ui.WindCard
import io.github.prejas3.windconnect.ui.WindSwitch

/** WN-50 Settings: str(R.string.find_allow) (default on). Off, the PC's "Ring" tile is disabled. */
@Composable
fun FindControls() {
    val connections = (LocalContext.current.applicationContext as WindApplication).connections
    val allowed by connections.findAllowed.collectAsStateWithLifecycle()
    WindSettingRow("target", str(R.string.find_channel), str(R.string.sum_find), checked = allowed, onCheckedChange = connections::setFindAllowed,
        switchTag = "findAllowed", more = { Text(str(R.string.find_about)) })
}

/** Shown over every screen while the phone rings for the PC; works without notifications. */
@Composable
fun StopRingingBar(onStop: () -> Unit, modifier: Modifier = Modifier) {
    Row(
        modifier.fillMaxWidth().statusBarsPadding().padding(12.dp)
            .background(MaterialTheme.colorScheme.errorContainer, RoundedCornerShape(16.dp))
            .padding(horizontal = 16.dp, vertical = 10.dp)
            .semantics { liveRegion = LiveRegionMode.Polite }
            .testTag("ringingBar"),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(str(R.string.find_ringing_now), color = MaterialTheme.colorScheme.onErrorContainer, modifier = Modifier.weight(1f))
        WindButton(str(R.string.find_stop), onStop, Modifier.testTag("stopRinging"))
    }
}
