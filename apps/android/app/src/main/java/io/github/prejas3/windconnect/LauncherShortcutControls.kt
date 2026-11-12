package io.github.prejas3.windconnect

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import io.github.prejas3.windconnect.ui.*

@Composable
fun LauncherShortcutControls() {
    val context = LocalContext.current
    var message by remember { mutableStateOf("") }
    WindCard {
        Text("Skróty na ekranie głównym", style = MaterialTheme.typography.titleMedium)
        Text("Przytrzymaj ikonę WindConnect w launcherze albo przypnij skrót poniżej. Pliki i touchpad są jeszcze niedostępne.")
        LauncherAction.entries.forEach { action ->
            val label = if (action == LauncherAction.Clipboard) action.title else "${action.title} (niedostępne)"
            WindButton("Przypnij: $label", { message = LauncherShortcuts.pin(context, action) },
                Modifier.testTag("pin_${action.id}"), kind = ButtonKind.Secondary)
        }
        if (message.isNotEmpty()) Text(message, Modifier.testTag("shortcutPinStatus"))
    }
}
