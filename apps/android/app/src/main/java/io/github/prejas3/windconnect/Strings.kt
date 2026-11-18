package io.github.prejas3.windconnect

import android.content.Context
import androidx.annotation.PluralsRes
import androidx.annotation.StringRes

/**
 * WN-63: UI text outside Compose (services, view models, notifications, widgets) comes from the app's resources:
 * English in values/, Polish in values-pl/, picked by the phone's (or the app's own) language.
 */
internal object Strings {
    @Volatile var context: Context? = null
    /** JVM unit tests have no resources: they read values-pl/strings.xml through these (StringsForTests). */
    @Volatile var lookup: ((Int, Array<out Any>) -> String)? = null
    @Volatile var pluralLookup: ((Int, Int, Array<out Any>) -> String)? = null
}

internal fun str(@StringRes id: Int, vararg args: Any): String =
    Strings.lookup?.invoke(id, args) ?: checkNotNull(Strings.context) { "WindApplication not created" }.getString(id, *args)

internal fun plural(@PluralsRes id: Int, count: Int, vararg args: Any): String =
    Strings.pluralLookup?.invoke(id, count, args)
        ?: checkNotNull(Strings.context) { "WindApplication not created" }.resources.getQuantityString(id, count, *args)
