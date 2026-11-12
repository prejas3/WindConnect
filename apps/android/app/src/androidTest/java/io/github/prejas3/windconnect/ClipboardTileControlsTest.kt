package io.github.prejas3.windconnect

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import io.github.prejas3.windconnect.ui.WindApp
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

class ClipboardTileControlsTest {
    @get:Rule val compose = createComposeRule()

    @Test fun olderAndroidShowsInstructionsWithoutRequestAction() {
        compose.setContent { WindApp(forceContent = true) {
            TileAddControls(32) { error("Older Android must not request a tile") }
        } }
        compose.onNodeWithTag("addClipboardTile").assertDoesNotExist()
        compose.onNodeWithTag("tileManualInstructions").assertIsDisplayed()
    }

    @Test fun explicitActionHandlesPendingRefusalSuccessAndErrorsWithoutNagging() {
        var requests = 0
        var complete: ((TileAddState) -> Unit)? = null
        compose.setContent { WindApp(forceContent = true) {
            TileAddControls(33) { requests++; complete = it }
        } }
        compose.runOnIdle { assertEquals(0, requests) }
        val action = compose.onNodeWithTag("addClipboardTile")
        action.performClick().assertIsNotEnabled()
        compose.runOnIdle { assertEquals(1, requests); complete!!(TileAddState.Declined) }
        compose.onNodeWithTag("tileAddStatus").assertTextEquals(TileAddState.Declined.message)
        action.assertIsEnabled()
        compose.runOnIdle { assertEquals(1, requests) }
        action.performClick()
        compose.runOnIdle { complete!!(TileAddState.Added) }
        compose.onNodeWithTag("tileAddStatus").assertTextEquals(TileAddState.Added.message)
        action.performClick()
        compose.runOnIdle { complete!!(TileAddState.Error) }
        compose.onNodeWithTag("tileAddStatus").assertTextEquals(TileAddState.Error.message)
        action.assertIsEnabled()
        compose.runOnIdle { assertEquals(3, requests) }
    }
}
