package io.github.prejas3.windconnect.core

import org.junit.Assert.assertEquals
import org.junit.Test

/** WN-159: the installed name first, then the one learned from notifications, the package name only as the last resort. */
class AppLabelsPickTest {
    private val pkg = "com.tippingcanoe.pepperpl"
    @Test fun installedWins() = assertEquals("Pepper", AppLabels.pick("Pepper", "Stara nazwa", pkg))
    @Test fun learnedWhenHidden() = assertEquals("Pepper", AppLabels.pick(null, "Pepper", pkg))
    @Test fun blankIsNoName() = assertEquals("Pepper", AppLabels.pick(" ", "Pepper", pkg))
    @Test fun packageNameLast() = assertEquals(pkg, AppLabels.pick(null, null, pkg))
}
