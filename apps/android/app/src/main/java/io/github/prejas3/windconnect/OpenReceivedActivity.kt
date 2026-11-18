package io.github.prejas3.windconnect

import android.content.ActivityNotFoundException
import android.content.Intent
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.launch

/**
 * WN-130: the "file received" notification's tap opens the file itself. It carries only the receipt key; the URI comes
 * from the app's own receive journal (CCS/1 SEC: never a URI taken from the intent). No journal open (process restarted)
 * or the file gone: the received-files list instead, which explains it.
 */
class OpenReceivedActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val key = intent.getStringExtra(KEY)
        lifecycleScope.launch {
            val uri = key?.let { (testUri ?: { k -> FileReceiving.openUri(k) })(it) }
            if (uri == null) startActivity(Intent(this@OpenReceivedActivity, ReceiveFilesActivity::class.java))
            else try {
                startActivity(Intent(Intent.ACTION_VIEW).setDataAndType(uri, contentResolver.getType(uri) ?: "application/octet-stream")
                    .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION))
            } catch (_: ActivityNotFoundException) {
                Toast.makeText(applicationContext, str(R.string.receive_no_app), Toast.LENGTH_SHORT).show()
                startActivity(Intent(this@OpenReceivedActivity, ReceiveFilesActivity::class.java))
            }
            finish()
        }
    }
    companion object {
        const val KEY = "receipt"
        /** Tests only: the journal lookup (no core opened in an instrumented test). */
        @androidx.annotation.VisibleForTesting @JvmStatic var testUri: (suspend (String) -> android.net.Uri?)? = null
    }
}
