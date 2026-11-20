package io.github.prejas3.windconnect

import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.heading

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import io.github.prejas3.windconnect.ui.*
import androidx.compose.ui.unit.dp

@Composable
fun LauncherShortcutControls() {
    val context = LocalContext.current
    var message by remember { mutableStateOf("") }
    WindSettingRow("plus", str(R.string.shortcuts_title), str(R.string.sum_shortcuts), status = message.ifEmpty { null }, statusTag = "shortcutPinStatus",
        actions = {
            // WN-160: one button per shortcut, the same width as every settings button.
            LauncherAction.entries.forEach { action ->
                WindButton(action.title, { message = LauncherShortcuts.pin(context, action) }, Modifier.testTag("pin_${action.id}"), kind = ButtonKind.Secondary)
            }
        },
        more = { Text(str(R.string.shortcuts_about)) })
}
