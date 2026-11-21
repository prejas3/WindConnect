package io.github.prejas3.windconnect

import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp
import io.github.prejas3.windconnect.ui.BrandConnectingDark
import io.github.prejas3.windconnect.ui.BrandConnectingLight
import io.github.prejas3.windconnect.ui.BrandDark
import io.github.prejas3.windconnect.ui.BrandLight
import io.github.prejas3.windconnect.ui.WindTypography
import io.github.prejas3.windconnect.ui.connectingColor
import io.github.prejas3.windconnect.ui.contrast
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** WN-171: the M3 type scale, the amber "connecting" (Szymon's decision), the grouped SAS code and fingerprint. */
class PolishDetailsTest {
    @Test fun typeScaleIsM3s() {
        assertEquals(22.sp, WindTypography.titleLarge.fontSize)
        assertEquals(24.sp, WindTypography.headlineSmall.fontSize); assertEquals(32.sp, WindTypography.headlineSmall.lineHeight)
        assertEquals(28.sp, WindTypography.headlineMedium.fontSize)
        assertEquals(45.sp, WindTypography.displayMedium.fontSize)
        for (style in listOf(WindTypography.displayLarge, WindTypography.displayMedium, WindTypography.displaySmall,
            WindTypography.headlineLarge, WindTypography.headlineMedium, WindTypography.headlineSmall)) assertEquals(FontWeight.SemiBold, style.fontWeight)
    }

    @Test fun connectingIsTheWindowsTrayAmber() {
        assertEquals(BrandConnectingLight, connectingColor(BrandLight))
        assertEquals(BrandConnectingDark, connectingColor(BrandDark))
        // As the tray's "searching" (generate-icons: #805015 / #efbc7c), and visible on the card.
        assertEquals(0xFF805015.toInt(), BrandConnectingLight.value.shr(32).toInt())
        assertTrue(contrast(BrandConnectingLight, BrandLight.surfaceContainerHigh) >= 3.0)
        assertTrue(contrast(BrandConnectingDark, BrandDark.surfaceContainerHigh) >= 3.0)
    }

    @Test fun codeAndFingerprintAreGrouped() {
        assertEquals("123 456", groupCode("123456"))
        assertEquals("1234", groupCode("1234"))
        assertEquals("ab12 cd34 ef", groupFingerprint("ab12cd34ef"))
        assertEquals("AB:CD", groupFingerprint("AB:CD"))
    }
}
