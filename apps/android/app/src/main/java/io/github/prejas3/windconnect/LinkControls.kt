package io.github.prejas3.windconnect

import io.github.prejas3.windconnect.ui.*
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

/** WN-100 Settings: str(R.string.links_allow) (default on). Off, the PC's "Open on the phone" is refused. */
@Composable
fun LinkControls() {
    val connections = (LocalContext.current.applicationContext as WindApplication).connections
    val allowed by connections.openLinks.collectAsStateWithLifecycle()
    WindSettingRow("external", str(R.string.link_channel), str(R.string.sum_links), checked = allowed, onCheckedChange = connections::setOpenLinks,
        switchTag = "openLinks", more = { Text(str(R.string.links_about)) })
}
