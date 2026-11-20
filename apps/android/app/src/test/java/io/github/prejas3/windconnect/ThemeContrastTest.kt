package io.github.prejas3.windconnect

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

/** WN-160: secondary text, accents and lines on a card reach a sure contrast, whatever the palette. */
class ThemeContrastTest {
    // A Samsung-like Material You dark scheme: grey cards, grey text on them.
    private val greyDark = darkColorScheme(
        background = Color(0xFF1B1B1F), surface = Color(0xFF1B1B1F), onSurface = Color(0xFFE4E1E6),
        surfaceVariant = Color(0xFF46464F), onSurfaceVariant = Color(0xFF9A99A3), surfaceContainerHigh = Color(0xFF2A2A2F),
        primary = Color(0xFF3F51B5), outline = Color(0xFF34343A), error = Color(0xFF93000A),
    )

    @Test fun dynamicCardsAreCalmAndTextReadable() {
        val scheme = readable(greyDark, dynamic = true)
        assertEquals("cards take surfaceContainerHigh", Color(0xFF2A2A2F), scheme.surfaceVariant)
        assertTrue(contrast(scheme.onSurfaceVariant, scheme.surfaceVariant) >= 7.0)
        assertTrue(contrast(scheme.primary, scheme.surfaceVariant) >= 4.5)
        assertTrue(contrast(scheme.outline, scheme.surfaceVariant) >= 1.8)
        assertTrue(contrast(scheme.error, scheme.surfaceVariant) >= 4.5)
    }

    @Test fun brandPalettesKeepTheirCardsAndPassToo() {
        for (brand in listOf(BrandLight, BrandDark)) {
            val scheme = readable(brand, dynamic = false)
            assertEquals(brand.surfaceVariant, scheme.surfaceVariant)
            assertEquals("main text untouched", brand.onSurface, scheme.onSurface)
            assertTrue(contrast(scheme.onSurfaceVariant, scheme.surfaceVariant) >= 7.0)
            assertTrue(contrast(scheme.primary, scheme.surfaceVariant) >= 4.5)
        }
    }

    @Test fun alreadyReadableColoursStayAsTheyAre() {
        val fine = lightColorScheme(surfaceVariant = Color.White, onSurfaceVariant = Color.Black, primary = Color(0xFF00007F),
            outline = Color(0xFF555555), error = Color(0xFF8B0000))
        val scheme = readable(fine, dynamic = false)
        assertEquals(Color.Black, scheme.onSurfaceVariant)
        assertEquals(Color(0xFF00007F), scheme.primary)
    }
}
