package io.github.prejas3.windconnect.core

import android.content.Context
import android.content.pm.PackageManager

/**
 * WN-124 (decision D2): whether this APK declares [permission] in its merged manifest. The Google
 * Play build (`-PwindStore=play`) leaves out the SMS, Call Log and contacts permissions; the
 * features that need them (SMS on the PC, calls on the PC) follow from this, so the manifest is the
 * one place that decides, and a feature is never offered without its permission.
 */
fun Context.declaresPermission(permission: String): Boolean = try {
    packageManager.getPackageInfo(packageName, PackageManager.GET_PERMISSIONS)
        .requestedPermissions?.contains(permission) == true
} catch (_: PackageManager.NameNotFoundException) {
    false
}
