package io.github.prejas3.windconnect

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.glance.appwidget.GlanceAppWidgetManager
import io.github.prejas3.windconnect.ui.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

@Composable
fun WidgetControls() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var message by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    WindCard {
        Text(str(R.string.widget_title), style = MaterialTheme.typography.titleMedium)
        Text(str(R.string.widget_about))
        WindButton(str(R.string.widget_add), {
            busy = true
            scope.launch {
                try {
                    val accepted = GlanceAppWidgetManager(context).requestPinGlanceAppWidget(WindWidgetReceiver::class.java, preview = WindWidget())
                    message = if (accepted) str(R.string.widget_requested)
                    else str(R.string.widget_unsupported)
                } catch (error: Exception) {
                    if (error is CancellationException) throw error
                    message = str(R.string.widget_failed)
                } finally { busy = false }
            }
        }, Modifier.testTag("addWidget"), enabled = !busy, kind = ButtonKind.Secondary)
        if (message.isNotEmpty()) Text(message, Modifier.testTag("widgetAddStatus"))
    }
}
