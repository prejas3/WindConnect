package io.github.prejas3.windconnect.core

import android.app.Notification
import android.content.Context
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.util.LruCache

/**
 * WN-159: apps' names (and icons) without seeing every installed app. Since Android 11 package visibility hides other apps
 * from [PackageManager.getApplicationInfo], and WindConnect asks for no QUERY_ALL_PACKAGES, so the lookup by package name
 * failed and the PC showed "com.tippingcanoe.pepperpl". Every notification carries its app's [ApplicationInfo] in its extras
 * ("android.appInfo", put there by Notification.Builder); its label and icon load from it. Labels learned that way are kept
 * (a small preferences file), so the media card and the settings' app list have them too.
 * SEC:the info only counts for its own package (an app can't name another one), and a lookup that works wins over it.
 */
internal object AppLabels {
    private const val FILE = "app-labels"
    private const val MAX = 300
    private const val EXTRA_APP_INFO = "android.appInfo"
    private val infos = LruCache<String, ApplicationInfo>(64)

    /** The app's info a notification carries, when it is for [packageName]. */
    fun info(notification: Notification?, packageName: String): ApplicationInfo? = try {
        @Suppress("DEPRECATION")
        (notification?.extras?.getParcelable<ApplicationInfo>(EXTRA_APP_INFO))?.takeIf { it.packageName == packageName }
    } catch (_: Exception) { null }

    /** Learns [packageName]'s label (and keeps its info for the icon) from a notification's [info]. */
    fun learn(context: Context, packageName: String, info: ApplicationInfo?) {
        if (info == null || info.packageName != packageName) return
        infos.put(packageName, info)
        val prefs = context.getSharedPreferences(FILE, Context.MODE_PRIVATE)
        val label = labelOf(context.packageManager, info, packageName) ?: return
        if (prefs.getString(packageName, null) == label) return
        val edit = prefs.edit()
        val known = prefs.all.keys
        if (known.size >= MAX && packageName !in known) known.take(known.size - MAX + 1).forEach { edit.remove(it) }
        edit.putString(packageName, label).apply()
    }

    /** The app's name: installed lookup, else learned from its notifications, else the package name. */
    fun label(context: Context, packageName: String): String {
        val pm = context.packageManager
        val installed = try { labelOf(pm, pm.getApplicationInfo(packageName, 0), packageName) } catch (_: Exception) { null }
        val learned = if (installed != null) null else infos.get(packageName)?.let { labelOf(pm, it, packageName) }
            ?: context.getSharedPreferences(FILE, Context.MODE_PRIVATE).getString(packageName, null)
        return pick(installed, learned, packageName)
    }

    /** The info to load an icon from: installed lookup, else the one a notification carried. */
    fun infoFor(context: Context, packageName: String): ApplicationInfo? =
        try { context.packageManager.getApplicationInfo(packageName, 0) } catch (_: Exception) { infos.get(packageName) }

    /** Android returns the package name for an app without a label: that is no label. */
    private fun labelOf(pm: PackageManager, info: ApplicationInfo, packageName: String): String? =
        try { pm.getApplicationLabel(info).toString().trim().takeIf { it.isNotEmpty() && it != packageName } } catch (_: Exception) { null }

    /** The first real name; the package name only when there is none. */
    fun pick(installed: String?, learned: String?, packageName: String): String =
        installed?.takeIf { it.isNotBlank() } ?: learned?.takeIf { it.isNotBlank() } ?: packageName

    /** Tests only. */
    fun forgetForTests(context: Context) { infos.evictAll(); context.getSharedPreferences(FILE, Context.MODE_PRIVATE).edit().clear().commit() }
}
