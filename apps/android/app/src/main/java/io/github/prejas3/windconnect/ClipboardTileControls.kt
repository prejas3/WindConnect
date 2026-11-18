package io.github.prejas3.windconnect

import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.heading

import android.app.StatusBarManager
import android.content.ComponentName
import android.graphics.drawable.Icon
import android.os.Build
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import io.github.prejas3.windconnect.ui.ButtonKind
import io.github.prejas3.windconnect.ui.WindButton
import io.github.prejas3.windconnect.ui.WindCard

internal enum class TileAddState(val message: String) {
    Idle(str(R.string.tile_add_about)),
    Pending(str(R.string.tile_confirm)),
    Added(str(R.string.tile_already)),
    Declined(str(R.string.tile_not_added)),
    Error(str(R.string.tile_failed)),
}

internal fun tileAddResult(result: Int): TileAddState = when (result) {
    StatusBarManager.TILE_ADD_REQUEST_RESULT_TILE_ADDED,
    StatusBarManager.TILE_ADD_REQUEST_RESULT_TILE_ALREADY_ADDED -> TileAddState.Added
    StatusBarManager.TILE_ADD_REQUEST_RESULT_TILE_NOT_ADDED -> TileAddState.Declined
    else -> TileAddState.Error
}

@Composable
fun ClipboardTileControls() {
    val context = LocalContext.current
    TileAddControls(Build.VERSION.SDK_INT) { complete ->
        if (Build.VERSION.SDK_INT >= 33) {
            try {
                context.getSystemService(StatusBarManager::class.java).requestAddTileService(
                    ComponentName(context, ClipboardTileService::class.java),
                    context.getString(R.string.tile_clipboard),
                    Icon.createWithResource(context, R.drawable.ic_connection),
                    context.mainExecutor,
                ) { result -> complete(tileAddResult(result)) }
            } catch (_: RuntimeException) {
                complete(TileAddState.Error)
            }
        }
    }
}

@Composable
internal fun TileAddControls(sdkInt: Int, requestAdd: ((TileAddState) -> Unit) -> Unit) {
    var state by remember { mutableStateOf(TileAddState.Idle) }
    WindCard {
        Text(str(R.string.tile_title), style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
        if (sdkInt >= 33) {
            Text(state.message, Modifier.testTag("tileAddStatus"))
            // CCS/1 RULE:only an explicit tap requests the OS prompt; refusal never triggers a retry.
            // WHY:keep the action after success so a tile removed from SystemUI can be added again.
            WindButton(str(R.string.tile_add), {
                state = TileAddState.Pending
                requestAdd { state = it }
            }, Modifier.testTag("addClipboardTile"), enabled = state != TileAddState.Pending, kind = ButtonKind.Secondary)
        } else {
            Text(str(R.string.tile_manual),
                Modifier.testTag("tileManualInstructions"))
        }
    }
}
