package io.github.prejas3.windconnect

import androidx.compose.foundation.layout.Column
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.activity.compose.setContent
import androidx.core.view.drawToBitmap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import io.github.prejas3.windconnect.ui.LocalReduceMotion
import io.github.prejas3.windconnect.ui.ThemeMode
import io.github.prejas3.windconnect.ui.WindConnectTheme
import io.github.prejas3.windconnect.ui.WindLinearProgress
import io.github.prejas3.windconnect.ui.WindSettingRow
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

/** WN-169: details expand, waits never freeze, and "Remove animations" means no motion. */
class MotionTest {
    @get:Rule val compose = createComposeRule()

    private fun show(reduce: Boolean, content: @Composable () -> Unit) = compose.setContent {
        WindConnectTheme(mode = ThemeMode.Light, dynamicColor = false) { CompositionLocalProvider(LocalReduceMotion provides reduce) { Column { content() } } }
    }

    private fun rowHeight() = compose.onNodeWithTag("row").fetchSemanticsNode().size.height

    private fun detailsOpen(reduce: Boolean): Pair<Int, Int> {
        show(reduce) { WindSettingRow("pc", "Row", "Description", Modifier.testTag("row"), more = { Text("Details\non two lines") }) }
        compose.waitForIdle()
        compose.mainClock.autoAdvance = false
        compose.onNodeWithTag("more:Row").performClick()
        compose.mainClock.advanceTimeByFrame(); compose.mainClock.advanceTimeByFrame()
        val early = rowHeight()
        compose.mainClock.advanceTimeBy(3_000)
        return early to rowHeight()
    }

    @Test fun detailsExpandOverTime() {
        val (early, settled) = detailsOpen(reduce = false)
        assertTrue("the row grows over frames ($early → $settled)", early < settled)
    }

    @Test fun detailsJumpWhenAnimationsAreOff() {
        val (early, settled) = detailsOpen(reduce = true)
        assertEquals("no motion: the end state at once", settled, early)
    }

    @Test fun unknownSizeIsAnnouncedAsIndeterminate() {
        show(reduce = false) { WindLinearProgress(null, Modifier.testTag("wait")) }
        compose.onNodeWithTag("wait").assert(SemanticsMatcher.expectValue(SemanticsProperties.ProgressBarRangeInfo, ProgressBarRangeInfo.Indeterminate))
    }
}
