package io.github.prejas3.windconnect

import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.heading

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.prejas3.windconnect.ui.WindCard
import io.github.prejas3.windconnect.ui.WindSwitch

/** WN-100 Settings: str(R.string.links_allow) (default on). Off, the PC's "Otwórz na telefonie" is refused. */
@Composable
fun LinkControls() {
    val connections = (LocalContext.current.applicationContext as WindApplication).connections
    val allowed by connections.openLinks.collectAsStateWithLifecycle()
    WindCard {
        Text(str(R.string.link_channel), style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
        WindSwitch(str(R.string.links_allow), allowed, connections::setOpenLinks, Modifier.testTag("openLinks"))
        Text(str(R.string.links_about),
            style = MaterialTheme.typography.bodySmall)
    }
}
