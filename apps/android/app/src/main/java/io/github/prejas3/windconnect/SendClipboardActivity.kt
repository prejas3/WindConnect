package io.github.prejas3.windconnect

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.lifecycleScope
import io.github.prejas3.windconnect.core.WindCore
import io.github.prejas3.windconnect.ui.*
import kotlinx.coroutines.launch

/** CCS/1 SEC:notification opens a focused window; no clipboard access on lock screen/in Service. */
class SendClipboardActivity : ComponentActivity() {
    private var core: WindCore? = null
    private var sent = false
    private var focused = false
    private val status = mutableStateOf("Przygotowywanie wysyłania schowka…")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        (application as WindApplication).connections.clipboardFocus(false)
        enableEdgeToEdge()
        setContent { WindApp(forceContent = true) {
            Column(Modifier.fillMaxSize().padding(24.dp), verticalArrangement = Arrangement.spacedBy(18.dp)) {
                Text("Wyślij schowek", style = MaterialTheme.typography.headlineSmall)
                Text(status.value)
                WindButton("Zamknij", { finish() }, kind = ButtonKind.Secondary)
            }
        } }
        lifecycleScope.launch {
            try { core = (application as WindApplication).connections.acquire(); sendIfFocused() }
            catch (_: Exception) { status.value = "Nie udało się otworzyć połączenia" }
        }
    }
    override fun onWindowFocusChanged(hasFocus: Boolean) { super.onWindowFocusChanged(hasFocus); focused = hasFocus; sendIfFocused() }
    private fun sendIfFocused() {
        val opened = core ?: return
        if (!focused || sent) return
        sent = true
        lifecycleScope.launch {
            try {
                val result = opened.sendCurrentClipboard()
                status.value = opened.clipboardUi.value.status
                if (result == "applied" || result == "unchanged") finish()
            } catch (_: Exception) { status.value = "Nie udało się wysłać schowka" }
        }
    }
    override fun onDestroy() {
        core?.let { opened -> kotlinx.coroutines.CoroutineScope(kotlinx.coroutines.Dispatchers.IO).launch { (application as WindApplication).connections.release(opened) } }
        core = null
        super.onDestroy()
    }
}
