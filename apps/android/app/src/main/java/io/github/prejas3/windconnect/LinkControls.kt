package io.github.prejas3.windconnect

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

/** WN-100 Settings: "Otwieraj linki z komputera" (default on). Off, the PC's "Otwórz na telefonie" is refused. */
@Composable
fun LinkControls() {
    val connections = (LocalContext.current.applicationContext as WindApplication).connections
    val allowed by connections.openLinks.collectAsStateWithLifecycle()
    WindCard {
        Text("Linki z komputera", style = MaterialTheme.typography.titleMedium)
        WindSwitch("Otwieraj linki z komputera", allowed, connections::setOpenLinks, Modifier.testTag("openLinks"))
        Text("Komputer może poprosić o otwarcie adresu http lub https. Gdy aplikacja jest na ekranie, link otwiera się od razu; " +
            "w tle pojawia się powiadomienie do stuknięcia. Nic innego komputer na telefonie nie uruchamia.",
            style = MaterialTheme.typography.bodySmall)
    }
}
