package io.github.prejas3.windconnect

import android.app.Application
import android.app.LocaleManager
import android.content.res.Configuration
import android.os.Build
import android.os.LocaleList
import androidx.test.runner.AndroidJUnitRunner
import java.util.Locale

/**
 * WN-63: the UI tests check the Polish texts, so the app under test runs in Polish whatever the emulator's language
 * (per-app language on Android 13+, the process configuration before that).
 */
class PolishTestRunner : AndroidJUnitRunner() {
    override fun callApplicationOnCreate(app: Application) {
        val polish = Locale.forLanguageTag("pl-PL")
        Locale.setDefault(polish)
        if (Build.VERSION.SDK_INT >= 33) app.getSystemService(LocaleManager::class.java).applicationLocales = LocaleList(polish)
        @Suppress("DEPRECATION")
        app.resources.updateConfiguration(Configuration(app.resources.configuration).apply { setLocale(polish) }, app.resources.displayMetrics)
        super.callApplicationOnCreate(app)
    }

    /** The per-app language is the device's setting: give it back (system default) once the run ends. */
    override fun finish(resultCode: Int, results: android.os.Bundle?) {
        if (Build.VERSION.SDK_INT >= 33) targetContext.getSystemService(LocaleManager::class.java).applicationLocales = LocaleList.getEmptyLocaleList()
        super.finish(resultCode, results)
    }
}
