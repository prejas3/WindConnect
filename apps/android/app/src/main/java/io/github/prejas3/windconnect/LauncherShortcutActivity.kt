package io.github.prejas3.windconnect

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.prejas3.windconnect.core.ConnectionState
import io.github.prejas3.windconnect.ui.*

/** CCS/1 SEC:exported fixed-action router, ignores all extras/URI; never reads clipboard or opens arbitrary intents. */
class LauncherShortcutActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val action = LauncherAction.fromAction(intent.action)
        if (action == null) { finish(); return }
        if (action == LauncherAction.Clipboard) {
            startActivity(Intent(this, SendClipboardActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK))
            finish()
            return
        }
        enableEdgeToEdge()
        setContent { WindApp(forceContent = true) {
            val state by (application as WindApplication).connections.state.collectAsStateWithLifecycle()
            Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp), verticalArrangement = Arrangement.spacedBy(18.dp)) {
                Text(action.title, style = MaterialTheme.typography.headlineSmall)
                Text("Ta funkcja jest jeszcze niedostępna w tej wersji WindConnect.", Modifier.testTag("shortcutUnavailable"))
                Text(if (state is ConnectionState.Connected) "Połączenie z komputerem jest aktywne, ale ta akcja nie jest jeszcze obsługiwana."
                    else "Brak aktywnego połączenia z komputerem. Ta akcja pozostaje niedostępna także po połączeniu.")
                WindButton("Zamknij", { finish() }, kind = ButtonKind.Secondary)
            }
        } }
    }
}
