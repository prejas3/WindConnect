package io.github.prejas3.windconnect

import android.service.quicksettings.Tile
import android.app.StatusBarManager
import io.github.prejas3.windconnect.core.ConnectionState
import org.junit.Assert.*
import org.junit.Test

class ClipboardTileTest {
    init { StringsForTests.useEnglish() }
    @Test fun addResultsDistinguishSuccessRefusalAndErrors() {
        assertEquals(TileAddState.Added, tileAddResult(StatusBarManager.TILE_ADD_REQUEST_RESULT_TILE_ADDED))
        assertEquals(TileAddState.Added, tileAddResult(StatusBarManager.TILE_ADD_REQUEST_RESULT_TILE_ALREADY_ADDED))
        assertEquals(TileAddState.Declined, tileAddResult(StatusBarManager.TILE_ADD_REQUEST_RESULT_TILE_NOT_ADDED))
        listOf(StatusBarManager.TILE_ADD_REQUEST_ERROR_APP_NOT_IN_FOREGROUND,
            StatusBarManager.TILE_ADD_REQUEST_ERROR_REQUEST_IN_PROGRESS,
            StatusBarManager.TILE_ADD_REQUEST_ERROR_BAD_COMPONENT, -1).forEach {
            assertEquals(TileAddState.Error, tileAddResult(it))
        }
    }

    @Test fun connectionLossNeverLeavesAnActiveTileOrExposesErrors() {
        val connected = clipboardTileState(ConnectionState.Connected("PC", "private-key", 50))
        assertEquals(Tile.STATE_ACTIVE, connected.first)
        assertEquals("Connected · PC", connected.second)
        listOf(ConnectionState.Ready, ConnectionState.Stopped,
            ConnectionState.Disconnected("private transport detail"),
            ConnectionState.Failed("private error")).forEach {
            assertEquals(Tile.STATE_INACTIVE to "Not connected", clipboardTileState(it))
        }
        assertEquals(Tile.STATE_INACTIVE to "Connecting…", clipboardTileState(ConnectionState.Connecting))
    }

    @Test fun longPeerNameIsBounded() {
        val state = clipboardTileState(ConnectionState.Connected("x".repeat(200), "", 1))
        assertEquals("Connected · " + "x".repeat(64), state.second)
    }
}
