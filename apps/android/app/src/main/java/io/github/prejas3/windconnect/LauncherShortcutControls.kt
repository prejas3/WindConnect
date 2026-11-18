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
            androidx.compose.foundation.layout.FlowRow(horizontalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(8.dp),
                verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(8.dp)) {
                LauncherAction.entries.forEach { action ->
                    WindButton(action.title, { message = LauncherShortcuts.pin(context, action) }, Modifier.testTag("pin_${action.id}"), kind = ButtonKind.Secondary)
                }
            }
        },
        more = { Text(str(R.string.shortcuts_about)) })
}
