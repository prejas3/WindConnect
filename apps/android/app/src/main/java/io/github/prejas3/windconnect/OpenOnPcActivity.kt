package io.github.prejas3.windconnect

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.widget.Toast
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/**
 * WN-100: "Otwórz na PC" in the share sheet. Takes the first http(s) link of shared text and asks the connected PC to open it
 * (§9.2 `open-url`); a toast says how it went. Without a link or a connection it says so and does nothing else.
 */
class OpenOnPcActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val link = PcActionText.link(intent?.takeIf { it.action == Intent.ACTION_SEND }?.getStringExtra(Intent.EXTRA_TEXT))
        if (link == null) {
            Toast.makeText(this, "Udostępniony tekst nie zawiera linku", Toast.LENGTH_SHORT).show()
            finish(); return
        }
        val app = applicationContext
        val connections = (application as WindApplication).connections
        CoroutineScope(SupervisorJob() + Dispatchers.Main).launch {
            // Only an existing connection: sharing never starts one (it would wait for discovery).
            val status = withTimeoutOrNull(8_000) { connections.runPcAction("open-url", link) } ?: "failed"
            Toast.makeText(app, PcActionText.of(status, "Otwórz na PC"), Toast.LENGTH_SHORT).show()
        }
        finish()
    }
}
