package io.github.prejas3.windconnect

import android.app.Application
import android.app.LocaleManager
import android.content.res.Configuration
import android.os.Build
import android.os.Bundle
import android.os.LocaleList
import androidx.test.runner.AndroidJUnitRunner
import java.util.Locale

/**
 * WN-63/WN-174: the app under test runs in one language whatever the emulator's: English, the source language, by
 * default; Polish with the instrumentation argument `lang=pl` (`-e lang pl`, or Gradle
 * `-Pandroid.testInstrumentationRunnerArguments.lang=pl`). Per-app language on Android 13+, the process configuration
 * before that.
 */
class LocaleTestRunner : AndroidJUnitRunner() {
    private var locale: Locale = Locale.ENGLISH

    override fun onCreate(arguments: Bundle?) {
        if (arguments?.getString("lang")?.lowercase() == "pl") locale = Locale.forLanguageTag("pl-PL")
        super.onCreate(arguments)
    }

    override fun callApplicationOnCreate(app: Application) {
        Locale.setDefault(locale)
        if (Build.VERSION.SDK_INT >= 33) app.getSystemService(LocaleManager::class.java).applicationLocales = LocaleList(locale)
        @Suppress("DEPRECATION")
        app.resources.updateConfiguration(Configuration(app.resources.configuration).apply { setLocale(locale) }, app.resources.displayMetrics)
        super.callApplicationOnCreate(app)
    }

    /** The per-app language is the device's setting: give it back (system default) once the run ends. */
    override fun finish(resultCode: Int, results: Bundle?) {
        if (Build.VERSION.SDK_INT >= 33) targetContext.getSystemService(LocaleManager::class.java).applicationLocales = LocaleList.getEmptyLocaleList()
        super.finish(resultCode, results)
    }
}
