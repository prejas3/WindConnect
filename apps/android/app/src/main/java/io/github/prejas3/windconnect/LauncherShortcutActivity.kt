package io.github.prejas3.windconnect

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity

/** CCS/1 SEC:exported fixed-action router, ignores all extras/URI; never reads clipboard or opens arbitrary intents. */
class LauncherShortcutActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        LauncherAction.fromAction(intent.action)?.let { startActivity(LauncherShortcuts.target(this, it)) }
        finish()
    }
}
