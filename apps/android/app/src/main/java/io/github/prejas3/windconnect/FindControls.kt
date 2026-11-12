package io.github.prejas3.windconnect

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

/** WN-50 Settings: "Pozwól komputerowi dzwonić" (default on). Off, the PC's "Dzwoń" tile is disabled. */
@Composable
fun FindControls() {
    val connections = (LocalContext.current.applicationContext as WindApplication).connections
    val allowed by connections.findAllowed.collectAsStateWithLifecycle()
    WindCard {
        Text("Znajdź telefon", style = MaterialTheme.typography.titleMedium)
        WindSwitch("Pozwól komputerowi dzwonić", allowed, connections::setFindAllowed, Modifier.testTag("findAllowed"))
        Text("Komputer może włączyć głośny dźwięk, także w trybie cichym, aż do 60 s. Zatrzymasz go w powiadomieniu, w aplikacji albo przyciskiem głośności przy otwartej aplikacji.", style = MaterialTheme.typography.bodySmall)
    }
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
        Text("Telefon dzwoni na prośbę komputera", color = MaterialTheme.colorScheme.onErrorContainer, modifier = Modifier.weight(1f))
        WindButton("Zatrzymaj dzwonienie", onStop, Modifier.testTag("stopRinging"))
    }
}
