package io.github.prejas3.windconnect.core

import android.Manifest
import android.app.ActivityManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import io.github.prejas3.windconnect.bindings.UrlOpener

/**
 * CCS/1 M:WN-100 a link from the PC (PROTOCOL.md §9.2 `open-url`), already checked by the core (absolute http/https, no user
 * info, printable ASCII). With WindConnect on screen it opens at once (ACTION_VIEW, the person's browser or app); otherwise
 * Android doesn't let a background app start an activity, so a notification "Open the link from the computer" offers it to tap.
 * RULE:only http(s) is ever opened (checked again here); nothing else from the PC reaches an Intent.
 */
class LinkOpener(context: Context) : UrlOpener {
    private val app = context.applicationContext

    override fun openUrl(url: String): Boolean {
        val uri = Uri.parse(url)
        if (uri.scheme?.lowercase() !in setOf("http", "https") || uri.host.isNullOrEmpty()) return false
        val view = Intent(Intent.ACTION_VIEW, uri).addCategory(Intent.CATEGORY_BROWSABLE).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if (visible()) {
            try { app.startActivity(view); return true } catch (_: ActivityNotFoundException) { return false } catch (_: SecurityException) {}
        }
        return offer(uri, view)
    }

    /** An activity of this app is in front (the only case Android lets it open another app). */
    private fun visible(): Boolean {
        val info = ActivityManager.RunningAppProcessInfo()
        ActivityManager.getMyMemoryState(info)
        return info.importance == ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND
    }

    private fun offer(uri: Uri, view: Intent): Boolean {
        if (Build.VERSION.SDK_INT >= 33 && app.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return false
        val manager = app.getSystemService(NotificationManager::class.java)
        if (!manager.areNotificationsEnabled()) return false
        manager.createNotificationChannel(NotificationChannel(CHANNEL, app.getString(R.string.link_channel), NotificationManager.IMPORTANCE_HIGH).apply {
            description = app.getString(R.string.link_channel_description)
        })
        val id = (System.currentTimeMillis() and 0x7fff_ffff).toInt()
        val open = PendingIntent.getActivity(app, id, view, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_ONE_SHOT)
        val host = uri.host ?: ""
        val notification = android.app.Notification.Builder(app, CHANNEL)
            .setSmallIcon(R.drawable.ic_link_open).setColor(0xFF0D7F74.toInt()) // WN-171: own icon, brand colour
            .setContentTitle(app.getString(R.string.link_open_title))
            .setContentText(host)
            .setStyle(android.app.Notification.BigTextStyle().bigText(SmsRules.clean(uri.toString()).take(300)))
            .setContentIntent(open)
            .setAutoCancel(true)
            .setCategory(android.app.Notification.CATEGORY_RECOMMENDATION)
            .build()
        manager.notify(TAG, id, notification)
        return true
    }

    companion object {
        const val CHANNEL = "links_from_pc"
        const val TAG = "link-from-pc"
    }
}
