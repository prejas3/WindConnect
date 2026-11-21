package io.github.prejas3.windconnect

import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.test.junit4.createComposeRule
import io.github.prejas3.windconnect.ui.BrandDark
import io.github.prejas3.windconnect.ui.BrandLight
import io.github.prejas3.windconnect.ui.ThemeMode
import io.github.prejas3.windconnect.ui.WindConnectTheme
import io.github.prejas3.windconnect.ui.WindShapes
import io.github.prejas3.windconnect.ui.readable
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Rule
import org.junit.Test

/**
 * WN-167: with Material You off (and on Android 10/11, where it doesn't exist) the running theme is the brand's in full:
 * the touchpad's field, the widget's "connecting" dot and the warning banners take brand roles, not the baseline M3
 * purple/pink, and the corner scale is MaterialTheme.shapes.
 */
class BrandThemeTest {
    @get:Rule val compose = createComposeRule()

    @Test fun materialYouOffGivesTheBrandSchemeAndShapes() {
        var scheme: ColorScheme? = null
        var shapes: Shapes? = null
        val mode = mutableStateOf(ThemeMode.Light)
        compose.setContent { WindConnectTheme(mode = mode.value, dynamicColor = false) { scheme = MaterialTheme.colorScheme; shapes = MaterialTheme.shapes } }
        for ((theme, brand) in listOf(ThemeMode.Light to BrandLight, ThemeMode.Dark to BrandDark)) {
            compose.runOnIdle { mode.value = theme }
            compose.runOnIdle {
                // ColorScheme has no equals of its own: its toString lists every role.
                assertEquals(readable(brand).toString(), scheme.toString())
                assertEquals(WindShapes.material, shapes)
                // The three places the audit saw in baseline colours (light/dark baseline values).
                for (baseline in listOf(Color(0xFFF3EDF7), Color(0xFF211F26))) assertNotEquals("touchpad field", baseline, scheme!!.surfaceContainer)
                for (baseline in listOf(Color(0xFF7D5260), Color(0xFFEFB8C8))) assertNotEquals("connecting dot", baseline, scheme!!.tertiary)
                for (baseline in listOf(Color(0xFFF9DEDC), Color(0xFF8C1D18))) assertNotEquals("warning banner", baseline, scheme!!.errorContainer)
            }
        }
    }
}
