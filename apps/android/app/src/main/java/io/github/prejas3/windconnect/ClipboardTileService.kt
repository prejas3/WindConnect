package io.github.prejas3.windconnect

import android.app.PendingIntent
import android.annotation.SuppressLint
import android.content.Intent
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import io.github.prejas3.windconnect.core.ConnectionState
import kotlinx.coroutines.*

/** CCS/1 FLOW:SystemUI listening -> shared live state; click -> unlocked focused clipboard Activity. */
class ClipboardTileService : TileService() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var observation: Job? = null

    override fun onStartListening() {
        super.onStartListening()
        observation?.cancel()
        observation = scope.launch {
            (application as WindApplication).connections.state.collect { state ->
                qsTile?.apply {
                    val presentation = clipboardTileState(state)
                    this.state = presentation.first
                    label = getString(R.string.tile_clipboard)
                    subtitle = presentation.second
                    contentDescription = getString(R.string.tile_description, label, subtitle)
                    if (Build.VERSION.SDK_INT >= 30) stateDescription = subtitle
                    updateTile()
                }
            }
        }
    }

    override fun onStopListening() {
        observation?.cancel()
        observation = null
        super.onStopListening()
    }

    // CCS/1 WHY:Intent overload is required on API29–33; PendingIntent overload exists only on API34+.
    @SuppressLint("StartActivityAndCollapseDeprecated")
    override fun onClick() {
        super.onClick()
        unlockAndRun {
            val intent = Intent(this, SendClipboardActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK)
            if (Build.VERSION.SDK_INT >= 34) {
                startActivityAndCollapse(PendingIntent.getActivity(this, 32, intent,
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE))
            } else {
                @Suppress("DEPRECATION")
                startActivityAndCollapse(intent)
            }
        }
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }
}

// CCS/1 SEC:only connection status/name; never show clipboard contents or raw transport errors.
internal fun clipboardTileState(state: ConnectionState): Pair<Int, String> = when (state) {
    is ConnectionState.Connected -> Tile.STATE_ACTIVE to str(R.string.conn_connected_via, state.peerName.take(64))
    ConnectionState.Connecting -> Tile.STATE_INACTIVE to str(R.string.connecting)
    else -> Tile.STATE_INACTIVE to str(R.string.not_connected)
}
