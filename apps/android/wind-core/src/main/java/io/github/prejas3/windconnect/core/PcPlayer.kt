package io.github.prejas3.windconnect.core

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.SystemClock
import io.github.prejas3.windconnect.bindings.MediaNow
import io.github.prejas3.windconnect.bindings.PcMediaSink
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.lang.ref.WeakReference

/** The PC's player as the app shows it (texts already cleaned). */
data class PcMedia(
    val title: String,
    val artist: String?,
    val album: String?,
    val app: String,
    val durationMs: Long?,
    val positionMs: Long,
    /** elapsedRealtime when [positionMs] was true: the UI extrapolates progress from it. */
    val atElapsed: Long,
    val playing: Boolean,
    val speedPermille: Long,
    /** §9.6 bits: 1 play, 2 pause, 4 next, 8 previous, 16 seek. */
    val actions: Long,
    val art: Bitmap?,
) {
    fun positionAt(elapsed: Long): Long {
        val moved = if (playing) (elapsed - atElapsed).coerceAtLeast(0) * speedPermille / 1000 else 0
        val at = positionMs + moved
        return durationMs?.let { at.coerceAtMost(it) } ?: at
    }
}

/**
 * CCS/1 M:WN-47 the PC's player on the phone (PROTOCOL.md §9.6, media/1 here): a StateFlow for the app's card, mirrored into the
 * phone's own MediaSession "WindConnect · <PC>" + a MediaStyle notification, so Android's media controls (notification, QS,
 * lock screen) work; their buttons go back to the PC as commands.
 * RULE:the session plays no local audio and has no media button receiver (it must not take the headset's play/pause from the
 * phone's players); it is released and the notification cancelled on Gone, the end of the session, the setting off and at
 * start (a notification left by a dead process). Texts come from the PC's other apps: one line, no control/bidi characters.
 */
internal class PcPlayer(context: Context, private val pcName: () -> String?, private val send: (String, Long?) -> Unit) : PcMediaSink {
    private val context = context.applicationContext
    private val notifications = this.context.getSystemService(NotificationManager::class.java)
    private val mutable = MutableStateFlow<PcMedia?>(null)
    val state: StateFlow<PcMedia?> = mutable.asStateFlow()
    private var session: MediaSession? = null
    @Volatile private var artCache: Pair<Int, Bitmap?>? = null

    init {
        current = WeakReference(this)
        notifications.cancel(NOTIFICATION) // left by a process that died while showing it
    }

    override fun now(now: MediaNow, art: ByteArray?) {
        val shown = PcMedia(
            title = clean(now.title).ifEmpty { context.getString(R.string.media_untitled) },
            artist = now.artist?.let(::clean)?.takeIf { it.isNotEmpty() },
            album = now.album?.let(::clean)?.takeIf { it.isNotEmpty() },
            app = clean(now.app),
            durationMs = now.durationMs?.toLong(),
            positionMs = now.positionMs.toLong(),
            atElapsed = SystemClock.elapsedRealtime(),
            playing = now.playing,
            speedPermille = now.speedPermille.toLong(),
            actions = now.actions.toLong(),
            art = art?.let { bytes -> artCache?.takeIf { it.first == bytes.contentHashCode() }?.second ?: decode(bytes).also { artCache = bytes.contentHashCode() to it } },
        )
        mutable.value = shown
        mirror(shown)
    }

    override fun gone() = clear()

    /** No PC player any more (Gone, session end, setting off, shutdown). */
    @Synchronized fun clear() {
        mutable.value = null
        session?.let { it.isActive = false; it.release() }
        session = null
        notifications.cancel(NOTIFICATION)
    }

    /** Our mirror session's token while it exists (PhoneMedia leaves it out, WN-47). */
    val token: MediaSession.Token? get() = session?.sessionToken

    /** A button in the app or Android's media controls. */
    fun command(action: String, positionMs: Long? = null) = send(action, positionMs)

    @Synchronized private fun mirror(shown: PcMedia) {
        val s = session ?: MediaSession(context, "WindConnect PC").also { created ->
            // No media button receiver: headset/Bluetooth keys stay with the phone's own players.
            created.setMediaButtonReceiver(null)
            created.setCallback(object : MediaSession.Callback() {
                override fun onPlay() = command("play")
                override fun onPause() = command("pause")
                override fun onSkipToNext() = command("next")
                override fun onSkipToPrevious() = command("previous")
                override fun onSeekTo(pos: Long) = command("seek", pos)
            }, android.os.Handler(android.os.Looper.getMainLooper())) // the core calls us on threads without a Looper
            session = created
        }
        val title = shown.title
        val subtitle = listOfNotNull(shown.artist, pcName()?.let { context.getString(R.string.media_on_pc, it) }).joinToString(" · ")
        s.setMetadata(MediaMetadata.Builder()
            .putString(MediaMetadata.METADATA_KEY_TITLE, title)
            .putString(MediaMetadata.METADATA_KEY_ARTIST, shown.artist ?: "")
            .putString(MediaMetadata.METADATA_KEY_ALBUM, shown.album ?: "")
            .putLong(MediaMetadata.METADATA_KEY_DURATION, shown.durationMs ?: -1)
            .apply { shown.art?.let { putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, it) } }
            .build())
        s.setPlaybackState(PlaybackState.Builder()
            .setState(if (shown.playing) PlaybackState.STATE_PLAYING else PlaybackState.STATE_PAUSED, shown.positionMs,
                shown.speedPermille / 1000f, shown.atElapsed)
            .setActions(playbackActions(shown.actions))
            .build())
        s.isActive = true
        notify(s, shown, subtitle)
    }

    private fun notify(s: MediaSession, shown: PcMedia, subtitle: String) {
        notifications.createNotificationChannel(NotificationChannel(CHANNEL, context.getString(R.string.media_channel), NotificationManager.IMPORTANCE_LOW).apply {
            description = context.getString(R.string.media_channel_description)
            setSound(null, null)
            enableVibration(false)
        })
        fun action(icon: Int, label: String, name: String, request: Int) = Notification.Action.Builder(
            android.graphics.drawable.Icon.createWithResource(context, icon), label,
            PendingIntent.getBroadcast(context, request, Intent(context, PcMediaActionReceiver::class.java).setAction(name),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)).build()
        val can = { bit: Long -> shown.actions and bit != 0L }
        val actions = buildList {
            if (can(8)) add(action(android.R.drawable.ic_media_previous, context.getString(R.string.media_previous), "previous", 61))
            add(if (shown.playing) action(android.R.drawable.ic_media_pause, context.getString(R.string.media_pause), "pause", 62)
                else action(android.R.drawable.ic_media_play, context.getString(R.string.media_play), "play", 63))
            if (can(4)) add(action(android.R.drawable.ic_media_next, context.getString(R.string.media_next), "next", 64))
        }
        val icon = context.resources.getIdentifier("ic_connection", "drawable", context.packageName).takeIf { it != 0 } ?: android.R.drawable.ic_media_play
        val notification = Notification.Builder(context, CHANNEL)
            .setSmallIcon(icon)
            .setContentTitle(shown.title)
            .setContentText(subtitle)
            .setLargeIcon(shown.art)
            .setOnlyAlertOnce(true)
            .setVisibility(Notification.VISIBILITY_PUBLIC)
            .setStyle(Notification.MediaStyle().setMediaSession(s.sessionToken).setShowActionsInCompactView(*IntArray(actions.size) { it }))
            .apply { actions.forEach(::addAction) }
            .build()
        // Without POST_NOTIFICATIONS nothing shows; the app's card still works.
        try { notifications.notify(NOTIFICATION, notification) } catch (_: SecurityException) {}
    }

    companion object {
        const val CHANNEL = "pc_media"
        const val NOTIFICATION = 47
        @Volatile private var current: WeakReference<PcPlayer>? = null
        internal fun fromNotification(action: String) { current?.get()?.command(action) }

        /** One line for display: control, bidi and zero-width characters out, runs of space collapsed. */
        fun clean(text: String): String = buildString {
            var space = true
            for (c in text) {
                val ch = if (Character.isISOControl(c) || Character.isWhitespace(c) || deceptive(c)) ' ' else c
                if (ch == ' ') { if (space) continue; space = true } else space = false
                append(ch)
            }
        }.trimEnd().take(256)

        private fun deceptive(c: Char) = c in '\u200B'..'\u200F' || c in '\u202A'..'\u202E' || c in '\u2066'..'\u2069' || c == '\uFEFF'

        /** §9.6 bits back to PlaybackState actions. */
        fun playbackActions(bits: Long): Long {
            var a = 0L
            if (bits and 1 != 0L) a = a or PlaybackState.ACTION_PLAY
            if (bits and 2 != 0L) a = a or PlaybackState.ACTION_PAUSE
            if (bits and 3 == 3L) a = a or PlaybackState.ACTION_PLAY_PAUSE
            if (bits and 4 != 0L) a = a or PlaybackState.ACTION_SKIP_TO_NEXT
            if (bits and 8 != 0L) a = a or PlaybackState.ACTION_SKIP_TO_PREVIOUS
            if (bits and 16 != 0L) a = a or PlaybackState.ACTION_SEEK_TO
            return a
        }

        /** JPEG/PNG by magic bytes only, bounded to 512 px. */
        private fun decode(bytes: ByteArray): Bitmap? {
            val jpeg = bytes.size > 3 && bytes[0] == 0xFF.toByte() && bytes[1] == 0xD8.toByte() && bytes[2] == 0xFF.toByte()
            val png = bytes.size > 4 && bytes[0] == 0x89.toByte() && bytes[1] == 0x50.toByte() && bytes[2] == 0x4E.toByte() && bytes[3] == 0x47.toByte()
            if (!jpeg && !png) return null
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
            if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
            var sample = 1
            while (maxOf(bounds.outWidth, bounds.outHeight) / sample > 512) sample *= 2
            return BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
        }
    }
}

/** The PC media notification's buttons (not exported; only our PendingIntents reach it). */
class PcMediaActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        intent.action?.takeIf { it in setOf("play", "pause", "next", "previous") }?.let(PcPlayer::fromNotification)
    }
}
