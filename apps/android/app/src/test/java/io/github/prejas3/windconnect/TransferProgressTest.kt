package io.github.prejas3.windconnect

import org.junit.Assert.*
import org.junit.Test

class TransferProgressTest {
    init { StringsForTests.useEnglish() }
    @Test fun rateUsesFullWindowsAndResetsAfterPauseAndRetry() {
        val rate = TransferRate()
        assertNull(rate.update(0, 0, true))
        assertNull(rate.update(500, 500, true))
        assertEquals(1000L, rate.update(1000, 1000, true))
        assertEquals(1000L, rate.update(1100, 1050, true))
        assertNull(rate.update(1200, 1050, false))
        assertNull(rate.update(2000, 1050, true))
        assertEquals(1000L, rate.update(3000, 2050, true))
        assertNull(rate.update(3100, 0, true))
    }
    @Test fun retryExcludesReceiverConfirmedIndices() {
        // Selection is by original index; equal names/URIs never stand in for delivery confirmation.
        assertEquals(listOf("c", "d", "e"), FileRetry.remaining(listOf("a", "b", "c", "d", "e"), setOf(0, 1)))
    }
    @Test fun sizesFollowTheUiLanguage() {
        // WN-153: the decimal separator follows the language, as on Windows.
        assertEquals("1,5 MiB", transferBytes(1536L * 1024, java.util.Locale.forLanguageTag("pl")))
        assertEquals("1.5 MiB", transferBytes(1536L * 1024, java.util.Locale.ENGLISH))
        assertEquals("512 B", transferBytes(512, java.util.Locale.forLanguageTag("pl")))
    }
}
