package io.github.prejas3.windconnect

import androidx.activity.compose.setContent
import androidx.compose.runtime.CompositionLocalProvider
import androidx.core.view.drawToBitmap
import io.github.prejas3.windconnect.ui.LocalReduceMotion
import io.github.prejas3.windconnect.ui.ThemeMode
import io.github.prejas3.windconnect.ui.WindConnectTheme
import io.github.prejas3.windconnect.ui.WindLinearProgress
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** WN-169: an indeterminate progress runs on the real frame clock (no Compose test rule here, it pauses infinite animations). */
class ProgressMotionTest {
    /**
     * Two pictures of an indeterminate bar 400 ms apart, drawn by a plain activity: the Compose test clock never runs
     * infinite animations, so this one runs outside it, on the real frame clock.
     */
    private fun frames(reduce: Boolean): Boolean {
        androidx.test.core.app.ActivityScenario.launch(androidx.activity.ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { activity ->
                activity.setContent {
                    WindConnectTheme(mode = ThemeMode.Light, dynamicColor = false) {
                        CompositionLocalProvider(LocalReduceMotion provides reduce) { WindLinearProgress(null) }
                    }
                }
            }
            Thread.sleep(500)
            var first: android.graphics.Bitmap? = null
            scenario.onActivity { first = it.window.decorView.drawToBitmap() }
            Thread.sleep(400)
            var later: android.graphics.Bitmap? = null
            scenario.onActivity { later = it.window.decorView.drawToBitmap() }
            return first!!.sameAs(later)
        }
    }

    @Test fun indeterminateProgressRuns() { assertFalse("an unknown size moves, not a frozen bar", frames(reduce = false)) }

    @Test fun indeterminateProgressStandsStillWhenAnimationsAreOff() { assertTrue(frames(reduce = true)) }

}
