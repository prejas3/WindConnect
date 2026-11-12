package io.github.prejas3.windconnect.core

import android.app.Notification
import android.app.Person
import android.content.ComponentName
import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Bundle
import android.os.Parcelable
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.util.Base64
import android.util.LruCache
import io.github.prejas3.windconnect.bindings.PhoneNotification
import java.io.ByteArrayOutputStream
import java.lang.ref.WeakReference
import java.security.MessageDigest
import java.security.SecureRandom

/** What a notification is made of, read out of Android's types (so the rules below are plain and testable). */
internal data class NotificationParts(
    val packageName: String,
    val flags: Int,
    val category: String?,
    val visibility: Int,
    val template: String?,
    val conversationTitle: CharSequence?,
    val bigTitle: CharSequence?,
    val title: CharSequence?,
    /** MessagingStyle messages, oldest first: (sender, text). */
    val messages: List<Pair<CharSequence?, CharSequence?>>,
    val bigText: CharSequence?,
    val text: CharSequence?,
    val progressMax: Int,
    val progressIndeterminate: Boolean,
)

/** A notification as it goes to the PC, before the id and the app's label. */
internal data class NotificationContent(val title: String?, val text: String?, val progress: Boolean)

/**
 * CCS/1 M:WN-42 the phone's notifications for the PC (PROTOCOL.md §9.7). RULE:persistent ones (ongoing, foreground service,
 * no-clear), group summaries, media players (WN-46 covers them), VISIBILITY_SECRET, our own and empty ones are skipped; texts lose
 * spans, control and bidi characters, runs of whitespace, and are cut to the protocol's bytes. Android 15+ redacts sensitive
 * content (OTP) for listeners: what it gives is sent as is, never worked around.
 */
internal object NotificationRules {
    const val MAX_TITLE = 256
    const val MAX_TEXT = 1024
    const val MAX_APP = 64
    const val MAX_PACKAGE = 128

    private const val PERSISTENT = Notification.FLAG_ONGOING_EVENT or Notification.FLAG_FOREGROUND_SERVICE or Notification.FLAG_NO_CLEAR

    /** Null: not for the PC. */
    fun content(parts: NotificationParts, ownPackage: String): NotificationContent? {
        if (parts.packageName == ownPackage) return null
        if (parts.flags and PERSISTENT != 0) return null
        if (parts.flags and Notification.FLAG_GROUP_SUMMARY != 0) return null
        if (parts.visibility == Notification.VISIBILITY_SECRET) return null
        if (parts.category == Notification.CATEGORY_TRANSPORT || parts.template?.endsWith("MediaStyle") == true) return null
        val title = clean(parts.conversationTitle ?: parts.bigTitle ?: parts.title, MAX_TITLE)
        val last = parts.messages.lastOrNull { clean(it.second, MAX_TEXT) != null }
        val text = if (last != null) {
            val sender = clean(last.first, MAX_TITLE)
            val body = clean(last.second, MAX_TEXT)!!
            // A group chat shows who wrote; a one-to-one chat already has them as its title.
            clean(if (sender != null && sender != title) "$sender: $body" else body, MAX_TEXT)
        } else clean(parts.bigText ?: parts.text, MAX_TEXT)
        if (title == null && text == null) return null
        val progress = parts.progressMax > 0 || parts.progressIndeterminate || parts.category == Notification.CATEGORY_PROGRESS
        return NotificationContent(title, text, progress)
    }

    /** Plain text, one line, no control or bidi characters, at most [max] UTF-8 bytes; null when nothing is left. */
    fun clean(text: CharSequence?, max: Int): String? {
        if (text == null) return null
        val out = StringBuilder(text.length)
        var space = false
        for (c in text.toString()) {
            when {
                c.isWhitespace() || c == ' ' -> space = true
                Character.getType(c) == Character.CONTROL.toInt() || isBidi(c) -> {}
                else -> {
                    if (space && out.isNotEmpty()) out.append(' ')
                    space = false
                    out.append(c)
                }
            }
        }
        return PhoneMedia.bounded(out.toString(), max).takeIf { it.isNotEmpty() }
    }

    /** Marks and embeddings/overrides/isolates that could reorder text on the PC (ZWJ stays: emoji need it). */
    private fun isBidi(c: Char) = c == '‎' || c == '‏' || c == '؜' || c in '‪'..'‮' || c in '⁦'..'⁩'

    /** 16 bytes, stable for one notification on this phone; the salt keeps known apps' keys unguessable on the PC. */
    fun id(key: String, salt: ByteArray): ByteArray =
        MessageDigest.getInstance("SHA-256").apply { update(salt); update(key.toByteArray()) }.digest().copyOf(16)
}

/** Reads a [StatusBarNotification] into [NotificationParts]. */
internal fun StatusBarNotification.parts(): NotificationParts {
    val n = notification
    val extras = n.extras ?: Bundle()
    val messages = (extras.getParcelableArray(Notification.EXTRA_MESSAGES) ?: emptyArray<Parcelable>()).mapNotNull { item ->
        val bundle = item as? Bundle ?: return@mapNotNull null
        @Suppress("DEPRECATION")
        val sender = bundle.getCharSequence("sender") ?: (bundle.getParcelable("sender_person") as? Person)?.name
        sender to bundle.getCharSequence("text")
    }
    return NotificationParts(
        packageName = packageName,
        flags = n.flags,
        category = n.category,
        visibility = n.visibility,
        template = extras.getString(Notification.EXTRA_TEMPLATE),
        conversationTitle = extras.getCharSequence(Notification.EXTRA_CONVERSATION_TITLE),
        bigTitle = extras.getCharSequence(Notification.EXTRA_TITLE_BIG),
        title = extras.getCharSequence(Notification.EXTRA_TITLE),
        messages = messages,
        bigText = extras.getCharSequence(Notification.EXTRA_BIG_TEXT),
        text = extras.getCharSequence(Notification.EXTRA_TEXT),
        progressMax = extras.getInt(Notification.EXTRA_PROGRESS_MAX, 0),
        progressIndeterminate = extras.getBoolean(Notification.EXTRA_PROGRESS_INDETERMINATE, false),
    )
}

/** A change for the PC: a notification with its app icon (PNG), or one gone. */
internal sealed interface NotificationEvent {
    /** [posted]: SystemClock.elapsedRealtime when the listener got it (for the latency proof). */
    data class Posted(val notification: PhoneNotification, val packageName: String, val posted: Long) : NotificationEvent
    data class Removed(val id: ByteArray) : NotificationEvent
}

/** App labels and 64 px PNG icons, per package, made once. */
internal class AppIcons(context: Context) {
    private val pm = context.applicationContext.packageManager
    private val labels = LruCache<String, String>(64)
    private val icons = LruCache<String, ByteArray>(64)

    fun label(packageName: String): String = labels.get(packageName) ?: (try {
        pm.getApplicationLabel(pm.getApplicationInfo(packageName, 0)).toString()
    } catch (_: Exception) { packageName }).let { NotificationRules.clean(it, NotificationRules.MAX_APP) ?: PhoneMedia.bounded(packageName, NotificationRules.MAX_APP) }
        .also { labels.put(packageName, it) }

    fun icon(packageName: String): ByteArray? = icons.get(packageName) ?: try {
        val drawable = pm.getApplicationIcon(packageName)
        val bitmap = Bitmap.createBitmap(64, 64, Bitmap.Config.ARGB_8888)
        drawable.setBounds(0, 0, 64, 64)
        drawable.draw(Canvas(bitmap))
        val out = ByteArrayOutputStream()
        bitmap.compress(Bitmap.CompressFormat.PNG, 100, out)
        bitmap.recycle()
        out.toByteArray().takeIf { it.size <= 16 * 1024 }?.also { icons.put(packageName, it) }
    } catch (_: Exception) { null }
}

/**
 * CCS/1 RULE:reads notifications for the PC only with its own consent (§9.6 rule 6): Android's access to this component (its own entry,
 * not MediaListenerService's) and the setting "Powiadomienia na komputerze", off by default. Off, it unbinds itself and reads
 * nothing. Notifications from before a session aren't sent (§9.7); only what comes while [sink] is set.
 */
class NotifyListenerService : NotificationListenerService() {
    override fun onListenerConnected() {
        current = WeakReference(this)
        if (!allowed(this)) requestUnbind()
    }
    override fun onListenerDisconnected() { current = null }

    override fun onNotificationPosted(sbn: StatusBarNotification) {
        val at = android.os.SystemClock.elapsedRealtime()
        val deliver = sink ?: return
        if (!allowed(this)) return
        val content = NotificationRules.content(sbn.parts(), packageName) ?: return
        val id = NotificationRules.id(sbn.key, salt(this))
        deliver(NotificationEvent.Posted(PhoneNotification(
            id = id,
            app = "",
            packageName = PhoneMedia.bounded(sbn.packageName, NotificationRules.MAX_PACKAGE),
            title = content.title,
            text = content.text,
            timeMs = sbn.postTime.toULong(),
            progress = content.progress,
        ), sbn.packageName, at))
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification) {
        val deliver = sink ?: return
        if (!allowed(this)) return
        deliver(NotificationEvent.Removed(NotificationRules.id(sbn.key, salt(this))))
    }

    companion object {
        @Volatile private var current: WeakReference<NotifyListenerService>? = null
        /** Set by [WindCore] while a session may take notifications. */
        @Volatile internal var sink: ((NotificationEvent) -> Unit)? = null
        @Volatile private var cachedSalt: ByteArray? = null

        fun granted(context: Context): Boolean =
            context.getSystemService(android.app.NotificationManager::class.java)
                .isNotificationListenerAccessGranted(ComponentName(context, NotifyListenerService::class.java))

        internal fun allowed(context: Context) =
            context.getSharedPreferences(WindCore.SETTINGS_FILE, Context.MODE_PRIVATE).getBoolean(WindCore.NOTIFY_ALLOWED, false)

        /** A random per-install salt for ids (WN-42 review, point 5). */
        @Synchronized internal fun salt(context: Context): ByteArray {
            cachedSalt?.let { return it }
            val prefs = context.getSharedPreferences(WindCore.SETTINGS_FILE, Context.MODE_PRIVATE)
            val saved = prefs.getString(NOTIFY_SALT, null)?.let { Base64.decode(it, Base64.NO_WRAP) }?.takeIf { it.size == 16 }
            val salt = saved ?: ByteArray(16).also { SecureRandom().nextBytes(it) }
                .also { prefs.edit().putString(NOTIFY_SALT, Base64.encodeToString(it, Base64.NO_WRAP)).commit() }
            cachedSalt = salt
            return salt
        }

        private const val NOTIFY_SALT = "notify_salt"

        /** Follows the setting: unbind when off, ask for a rebind when on. */
        fun follow(context: Context, allowed: Boolean) {
            if (!granted(context)) return
            if (allowed) requestRebind(ComponentName(context, NotifyListenerService::class.java))
            else current?.get()?.requestUnbind()
        }
    }
}
