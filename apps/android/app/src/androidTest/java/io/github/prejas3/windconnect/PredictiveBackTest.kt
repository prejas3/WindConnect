package io.github.prejas3.windconnect

import androidx.activity.BackEventCompat
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import org.junit.Rule
import org.junit.Test

/** WN-169: back is predictive: a swipe previews leaving, cancelling keeps the screen, letting go goes back. */
class PredictiveBackTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()

    @Test fun backPreviewsAndCancelsBeforeLeavingSettings() {
        compose.waitUntil(15_000) { compose.onAllNodesWithTag("openSettings").fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithTag("openSettings").performClick()
        compose.onNodeWithTag("closeSettings").assertExists()
        val back = compose.activity.onBackPressedDispatcher
        // A swipe started and taken back: still in Settings.
        compose.runOnUiThread {
            back.dispatchOnBackStarted(BackEventCompat(0f, 500f, 0f, BackEventCompat.EDGE_LEFT))
            back.dispatchOnBackProgressed(BackEventCompat(120f, 500f, .5f, BackEventCompat.EDGE_LEFT))
            back.dispatchOnBackCancelled()
        }
        compose.waitForIdle()
        compose.onNodeWithTag("closeSettings").assertExists()
        // A swipe let go: back on the home screen.
        compose.runOnUiThread {
            back.dispatchOnBackStarted(BackEventCompat(0f, 500f, 0f, BackEventCompat.EDGE_LEFT))
            back.dispatchOnBackProgressed(BackEventCompat(200f, 500f, .8f, BackEventCompat.EDGE_LEFT))
            back.onBackPressed()
        }
        compose.waitUntil(5_000) { compose.onAllNodesWithTag("closeSettings").fetchSemanticsNodes().isEmpty() }
        compose.onNodeWithTag("openSettings").assertExists()
    }
}
