package io.github.prejas3.windconnect.core

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class GalleryRulesTest {
    @Test fun screenshotsAreTheScreenshotsFolder() {
        assertTrue(GalleryRules.isScreenshot("DCIM/Screenshots/"))
        assertTrue(GalleryRules.isScreenshot("Pictures/Screenshots/"))
        assertTrue(GalleryRules.isScreenshot("Pictures/screenshots"))
        assertFalse(GalleryRules.isScreenshot("DCIM/Camera/"))
        assertFalse(GalleryRules.isScreenshot("Pictures/MyScreenshotsBackup/"))
        assertFalse(GalleryRules.isScreenshot(null))
    }
}
