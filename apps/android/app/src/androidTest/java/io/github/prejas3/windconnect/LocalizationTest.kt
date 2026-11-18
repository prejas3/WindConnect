package io.github.prejas3.windconnect

import android.content.Context
import android.content.res.Configuration
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.util.Locale
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/** WN-63: the installed APK resolves every text in English and Polish (values / values-pl, both modules). */
@RunWith(AndroidJUnit4::class)
class LocalizationTest {
    private val app: Context = InstrumentationRegistry.getInstrumentation().targetContext

    private fun localized(tag: String): Context =
        app.createConfigurationContext(Configuration(app.resources.configuration).apply { setLocale(Locale.forLanguageTag(tag)) })

    @Test fun everyTextExistsInBothLanguages() {
        val en = localized("en-US"); val pl = localized("pl-PL")
        val ids = R.string::class.java.fields.filter { it.name != "app_name" && it.name != "welcome_message" }.map { it.getInt(null) }
        var differ = 0
        for (id in ids) {
            val english = en.getString(id); val polish = pl.getString(id)
            assertTrue("empty ${app.resources.getResourceEntryName(id)}", english.isNotBlank() && polish.isNotBlank())
            if (english != polish) differ++
        }
        // Library strings come in too; most of ours differ between the languages.
        assertTrue("$differ of ${ids.size} differ", differ > 400)
    }

    @Test fun languagesAndPlurals() {
        val en = localized("en-US"); val pl = localized("pl-PL"); val de = localized("de-DE")
        assertEquals("Connected", en.getString(R.string.connected))
        assertEquals("Połączono", pl.getString(R.string.connected))
        // Anything but Polish gets English.
        assertEquals("Connected", de.getString(R.string.connected))
        assertEquals("Image sent to the computer", en.getString(io.github.prejas3.windconnect.core.R.string.clip_image_sent))
        assertEquals("Wysłano 5 plików do PC", pl.resources.getQuantityString(R.plurals.send_sent_to, 5, 5, "PC"))
        assertEquals("Wysłano 2 pliki do PC", pl.resources.getQuantityString(R.plurals.send_sent_to, 2, 2, "PC"))
        assertEquals("Sent 1 file to PC", en.resources.getQuantityString(R.plurals.send_sent_to, 1, 1, "PC"))
    }
}
