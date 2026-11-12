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
        Text("Widżet stanu PC", style = MaterialTheme.typography.titleMedium)
        Text("Stan połączenia, bateria telefonu i szybkie akcje. Możesz też dodać go z listy widżetów launchera.")
        WindButton("Dodaj widżet", {
            busy = true
            scope.launch {
                try {
                    val accepted = GlanceAppWidgetManager(context).requestPinGlanceAppWidget(WindWidgetReceiver::class.java, preview = WindWidget())
                    message = if (accepted) "Prośba o dodanie widżetu została przekazana do launchera. Potwierdź ją, jeśli pojawi się okno."
                    else "Dodaj WindConnect z listy widżetów launchera — ten launcher nie obsługuje próśb o przypięcie."
                } catch (error: Exception) {
                    if (error is CancellationException) throw error
                    message = "Nie udało się dodać widżetu. Spróbuj z listy widżetów launchera."
                } finally { busy = false }
            }
        }, Modifier.testTag("addWidget"), enabled = !busy, kind = ButtonKind.Secondary)
        if (message.isNotEmpty()) Text(message, Modifier.testTag("widgetAddStatus"))
    }
}
