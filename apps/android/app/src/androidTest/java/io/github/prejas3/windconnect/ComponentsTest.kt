package io.github.prejas3.windconnect

import android.os.Build
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Rule
import org.junit.Test
import org.junit.Assert.assertEquals
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.semantics.SemanticsActions
import io.github.prejas3.windconnect.ui.BrandDark
import io.github.prejas3.windconnect.ui.BrandLight
import androidx.core.view.WindowCompat
import org.junit.Assert.assertTrue
import androidx.compose.ui.text.TextMeasurer
import androidx.compose.ui.text.font.createFontFamilyResolver
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.sp
import io.github.prejas3.windconnect.ui.WindTypography

class ComponentsTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()

    @Test fun variableFontWeightsHaveDifferentRealGlyphMetrics() {
        compose.runOnIdle {
            val measurer = TextMeasurer(createFontFamilyResolver(compose.activity), Density(compose.activity), LayoutDirection.Ltr)
            val base = WindTypography.bodyLarge.copy(fontSize = 24.sp, lineHeight = 32.sp)
            val text = "WindConnect SZYMON-PC Zażółć 0123456789"
            val regular = measurer.measure(text, style = base.copy(fontWeight = FontWeight.Normal), softWrap = false).size.width
            val medium = measurer.measure(text, style = base.copy(fontWeight = FontWeight.Medium), softWrap = false).size.width
            val semibold = measurer.measure(text, style = base.copy(fontWeight = FontWeight.SemiBold), softWrap = false).size.width
            val bold = measurer.measure(text, style = base.copy(fontWeight = FontWeight.Bold), softWrap = false).size.width
            assertTrue("Actual glyph metrics must grow with wght: $regular/$medium/$semibold/$bold", regular < medium && medium < semibold && semibold < bold)
        }
    }

    @Test fun previewSupportsThemesDynamicColorActionsAndValidation() {
        compose.onNodeWithTag("previewToggle").performClick()
        compose.onNodeWithTag("componentPreview").assertExists()
        compose.onNodeWithTag("theme-Light").performClick().assertIsSelected()
        compose.onNodeWithTag("theme-Dark").performClick().assertIsSelected()
        if (Build.VERSION.SDK_INT >= 31) {
            val dynamic = compose.onNodeWithTag("dynamicColor")
            // CCS/1 RULE:preferences survive other tests; normalize before checking both sources.
            if (compose.onNodeWithTag("paletteSource").fetchSemanticsNode().config[androidx.compose.ui.semantics.SemanticsProperties.Text].first().text.contains("systemowy")) dynamic.performClick()
            compose.onNodeWithTag("paletteSource").assertTextContains("WN-20", substring = true)
            dynamic.performClick()
            compose.onNodeWithTag("paletteSource").assertTextContains("systemowy", substring = true)
            dynamic.performClick()
        }
        compose.onNodeWithTag("actionSend").performScrollTo().performClick()
        compose.onNodeWithTag("previewFeedback").performScrollTo().assertTextEquals("Przykład: wyślij plik")
        compose.onNodeWithTag("sampleSwitch").performScrollTo().assertIsOn().performClick().assertIsOff()
        compose.onNodeWithTag("validateName").performScrollTo().performClick()
        compose.onNodeWithText("Wpisz nazwę komputera.").assertExists()
        compose.onNodeWithTag("sampleName").performScrollTo().performTextInput("Zażółć gęślą jaźń")
        compose.onNodeWithTag("validateName").performScrollTo().performClick()
        compose.onNodeWithText("Wpisz nazwę komputera.").assertDoesNotExist()
        compose.onNodeWithText("Niedostępny").assertIsNotEnabled()
        compose.activityRule.scenario.recreate()
        compose.onNodeWithTag("componentPreview").assertExists()
        compose.onNodeWithTag("theme-Dark").performScrollTo().assertIsSelected()
        compose.onNodeWithTag("previewToggle").performClick()
        compose.onNodeWithTag("peerAddress").performScrollTo().assertExists()
        compose.onNodeWithTag("localKey").assertExists()
    }

    @Test fun captureBrandLightAndDarkPreview() {
        compose.onNodeWithTag("previewToggle").performClick()
        if (Build.VERSION.SDK_INT >= 31) {
            val source = compose.onNodeWithTag("paletteSource").fetchSemanticsNode().config[androidx.compose.ui.semantics.SemanticsProperties.Text].first().text
            if (source.contains("systemowy")) compose.onNodeWithTag("dynamicColor").performClick()
        }
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        for (theme in listOf("Light", "Dark")) {
            compose.onNodeWithTag("theme-$theme").performScrollTo().performClick()
            compose.waitForIdle()
            val title = compose.onNodeWithText("WindConnect").fetchSemanticsNode().boundsInRoot
            val toggle = compose.onNodeWithTag("previewToggle").fetchSemanticsNode().boundsInRoot
            assertTrue("Header must not overlap at larger font scales", title.right <= toggle.left)
            compose.runOnIdle {
                val controller = WindowCompat.getInsetsController(compose.activity.window, compose.activity.window.decorView)
                assertEquals(theme == "Light", controller.isAppearanceLightStatusBars)
                assertEquals(theme == "Light", controller.isAppearanceLightNavigationBars)
            }
            compose.onNodeWithText("Podgląd komponentów").performSemanticsAction(SemanticsActions.GetTextLayoutResult) { action ->
                val layouts = mutableListOf<TextLayoutResult>()
                action(layouts)
                assertEquals(if (theme == "Dark") BrandDark.onBackground else BrandLight.onBackground, layouts.single().layoutInput.style.color)
            }
            val screenshot = instrumentation.uiAutomation.takeScreenshot()
            val file = java.io.File(compose.activity.cacheDir, "wn22-$theme.png")
            file.outputStream().use { screenshot.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
            screenshot.recycle()
        }
    }
}
