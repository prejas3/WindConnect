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
        Text(str(R.string.shortcuts_title), style = MaterialTheme.typography.titleMedium)
        Text(str(R.string.shortcuts_about))
        LauncherAction.entries.forEach { action ->
            WindButton(str(R.string.shortcut_pin, action.title), { message = LauncherShortcuts.pin(context, action) },
                Modifier.testTag("pin_${action.id}"), kind = ButtonKind.Secondary)
        }
        if (message.isNotEmpty()) Text(message, Modifier.testTag("shortcutPinStatus"))
    }
}
