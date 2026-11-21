package io.github.prejas3.windconnect

import androidx.compose.material3.ColorScheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.ui.graphics.Color
import io.github.prejas3.windconnect.ui.BrandDark
import io.github.prejas3.windconnect.ui.BrandLight
import io.github.prejas3.windconnect.ui.contrast
import io.github.prejas3.windconnect.ui.readable
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * WN-160: secondary text, accents and lines on a card reach a sure contrast, whatever the palette.
 * WN-167: the brand schemes set every role (no baseline M3 purple or pink is left) and cards are surfaceContainerHigh.
 */
class ThemeContrastTest {
    // A Samsung-like Material You dark scheme: grey cards, grey text on them.
    private val greyDark = darkColorScheme(
        background = Color(0xFF1B1B1F), surface = Color(0xFF1B1B1F), onSurface = Color(0xFFE4E1E6),
        surfaceVariant = Color(0xFF46464F), onSurfaceVariant = Color(0xFF9A99A3), surfaceContainerHigh = Color(0xFF2A2A2F),
        primary = Color(0xFF3F51B5), outline = Color(0xFF34343A), outlineVariant = Color(0xFF2E2E33), error = Color(0xFF93000A),
    )

    /** Every Color role of a scheme by name (the getters of the inline class, `getPrimary-0d7_KjU`). */
    private fun roles(scheme: ColorScheme): Map<String, Color> = ColorScheme::class.java.methods
        .filter { it.name.startsWith("get") && it.parameterCount == 0 && it.returnType == java.lang.Long.TYPE }
        .associate { it.name.removePrefix("get").substringBefore('-') to Color((it.invoke(scheme) as Long).toULong()) }

    @Test fun dynamicCardsAreCalmAndTextReadable() {
        val scheme = readable(greyDark)
        val card = scheme.surfaceContainerHigh
        assertEquals("cards are surfaceContainerHigh, untouched", Color(0xFF2A2A2F), card)
        assertTrue(contrast(scheme.onSurfaceVariant, card) >= 7.0)
        assertTrue(contrast(scheme.primary, card) >= 4.5)
        assertTrue(contrast(scheme.outline, card) >= 1.8)
        assertTrue(contrast(scheme.outlineVariant, card) >= 1.5)
        assertTrue(contrast(scheme.error, card) >= 4.5)
    }

    @Test fun brandPalettesKeepTheirCardsAndPassToo() {
        for (brand in listOf(BrandLight, BrandDark)) {
            val scheme = readable(brand)
            val card = scheme.surfaceContainerHigh
            assertEquals("the WN-20 card colour", brand.surfaceVariant, card)
            assertEquals("main text untouched", brand.onSurface, scheme.onSurface)
            assertTrue(contrast(scheme.onSurfaceVariant, card) >= 7.0)
            assertTrue(contrast(scheme.primary, card) >= 4.5)
            assertTrue(contrast(scheme.outline, card) >= 1.8)
            assertTrue(contrast(scheme.outlineVariant, card) >= 1.5)
            // Dividers stay quieter than edges (WN-167, audit point 14).
            assertTrue(contrast(scheme.outlineVariant, card) < contrast(scheme.outline, card))
        }
    }

    @Test fun noBrandRoleIsABaselineM3Value() {
        for ((brand, baseline) in listOf(BrandLight to lightColorScheme(), BrandDark to darkColorScheme())) {
            val ours = roles(brand)
            val base = roles(baseline)
            assertTrue("reflection sees the roles (${ours.keys})", ours.size >= 36)
            // Pure white and black carry no hue: they may match the baseline (onPrimary, scrim, the lowest container).
            val leftovers = ours.filter { (role, color) -> color == base[role] && color != Color.White && color != Color.Black }
            assertTrue("roles left at the baseline M3 value: $leftovers", leftovers.isEmpty())
        }
    }

    @Test fun brandTextOnItsContainersIsReadable() {
        for (brand in listOf(BrandLight, BrandDark)) {
            val pairs = mapOf(
                "onPrimary" to (brand.onPrimary to brand.primary),
                "onPrimaryContainer" to (brand.onPrimaryContainer to brand.primaryContainer),
                "onSecondary" to (brand.onSecondary to brand.secondary),
                "onSecondaryContainer" to (brand.onSecondaryContainer to brand.secondaryContainer),
                "onTertiary" to (brand.onTertiary to brand.tertiary),
                "onTertiaryContainer" to (brand.onTertiaryContainer to brand.tertiaryContainer),
                "onError" to (brand.onError to brand.error),
                "onErrorContainer" to (brand.onErrorContainer to brand.errorContainer),
                "inverseOnSurface" to (brand.inverseOnSurface to brand.inverseSurface),
                "onSurface on every container" to (brand.onSurface to brand.surfaceContainerHighest),
            )
            for ((name, pair) in pairs) assertTrue("$name ${contrast(pair.first, pair.second)}", contrast(pair.first, pair.second) >= 4.5)
            // The warning banner (WindNotice, find-my-phone) is small text: a sure 7:1.
            assertTrue(contrast(brand.onErrorContainer, brand.errorContainer) >= 7.0)
        }
    }

    @Test fun alreadyReadableColoursStayAsTheyAre() {
        val fine = lightColorScheme(surfaceContainerHigh = Color.White, onSurfaceVariant = Color.Black, primary = Color(0xFF00007F),
            outline = Color(0xFF555555), outlineVariant = Color(0xFF777777), error = Color(0xFF8B0000))
        val scheme = readable(fine)
        assertEquals(Color.Black, scheme.onSurfaceVariant)
        assertEquals(Color(0xFF00007F), scheme.primary)
        assertEquals(Color(0xFF777777), scheme.outlineVariant)
    }
}
