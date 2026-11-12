package io.github.prejas3.windconnect.core

import android.content.ComponentName
import android.content.Context
import android.graphics.Bitmap
import android.media.MediaMetadata
import android.media.session.MediaController
import android.media.session.MediaSessionManager
import android.media.session.PlaybackState
import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import android.service.notification.NotificationListenerService
import io.github.prejas3.windconnect.bindings.MediaNow
import io.github.prejas3.windconnect.bindings.MediaSink
import java.io.ByteArrayOutputStream
import java.lang.ref.WeakReference
import kotlin.math.roundToLong

/**
 * CCS/1 M:WN-46 the phone's player for the PC (PROTOCOL.md §9.6). Reads other apps' MediaSessions, which Android gives only to
 * an enabled notification listener ([MediaListenerService]); it reads no notifications.
 * RULE:only while a session is live, access is granted and the person allows it; event-driven (session and controller
 * callbacks), never polled. Picks the first PLAYING session, else the first in the system's order.
 */
internal class PhoneMedia(
    context: Context,
    /** Our own session mirroring the PC's player (WN-47): never sent back to the PC. */
    private val ownSession: () -> android.media.session.MediaSession.Token? = { null },
    private val publish: (MediaNow?, ByteArray?) -> Unit,
) : MediaSink {
    private val context = context.applicationContext
    private val sessions = this.context.getSystemService(MediaSessionManager::class.java)
    private val component = ComponentName(this.context, MediaListenerService::class.java)
    private var thread: HandlerThread? = null
    private var handler: Handler? = null
    @Volatile private var controller: MediaController? = null
    private var art: Pair<Bitmap, ByteArray?>? = null

    private val callback = object : MediaController.Callback() {
        override fun onPlaybackStateChanged(state: PlaybackState?) = choose()
        override fun onMetadataChanged(metadata: MediaMetadata?) = report()
        override fun onSessionDestroyed() = choose()
    }
    private val changed = MediaSessionManager.OnActiveSessionsChangedListener { choose() }

    /** Starts following the players; false without notification-listener access. */
    @Synchronized fun start(): Boolean {
        if (thread != null) return true
        if (!MediaListenerService.granted(context)) return false
        val started = HandlerThread("WindConnect media").also { it.start() }
        val h = Handler(started.looper)
        try { sessions.addOnActiveSessionsChangedListener(changed, component, h) } catch (_: SecurityException) { started.quitSafely(); return false }
        thread = started; handler = h
        h.post { choose() }
        return true
    }

    @Synchronized fun stop() {
        val h = handler ?: return
        try { sessions.removeOnActiveSessionsChangedListener(changed) } catch (_: Exception) {}
        controller?.unregisterCallback(callback)
        controller = null
        art = null
        thread?.quitSafely()
        thread = null; handler = null
        h.removeCallbacksAndMessages(null)
    }

    /** On the media thread: follow the session [pick] chooses. */
    private fun choose() {
        // Never our own mirror of the PC's player (PcPlayer, WN-47): it would echo back to the PC. Matched by its token, so
        // other sessions in this process (none in the app; the instrumentation's test player) still count.
        val active = try { sessions.getActiveSessions(component) } catch (_: SecurityException) { emptyList() }
            .let { all -> eligible(all.map { it.sessionToken }, ownSession()).map { all[it] } }
        val chosen = pick(active.map { it.playbackState?.state }).let { active.getOrNull(it) }
        if (chosen?.sessionToken != controller?.sessionToken) {
            controller?.unregisterCallback(callback)
            controller = chosen
            art = null
            chosen?.registerCallback(callback, handler)
        }
        report()
    }

    /** On the media thread: what the PC should show now. */
    private fun report() {
        val c = controller
        val state = c?.playbackState
        if (c == null || state == null || state.state == PlaybackState.STATE_NONE || state.state == PlaybackState.STATE_STOPPED) {
            publish(null, null); return
        }
        val meta = c.metadata
        val label = try { context.packageManager.getApplicationLabel(context.packageManager.getApplicationInfo(c.packageName, 0)).toString() } catch (_: Exception) { c.packageName }
        val playing = state.state == PlaybackState.STATE_PLAYING
        val speed = (state.playbackSpeed * 1000).roundToLong().coerceAtLeast(0)
        val since = SystemClock.elapsedRealtime() - state.lastPositionUpdateTime
        val position = (state.position + if (playing && state.lastPositionUpdateTime > 0) since * speed / 1000 else 0).coerceAtLeast(0)
        val duration = meta?.getLong(MediaMetadata.METADATA_KEY_DURATION)?.takeIf { it > 0 }
        val now = MediaNow(
            title = bounded(meta?.getString(MediaMetadata.METADATA_KEY_TITLE) ?: meta?.getString(MediaMetadata.METADATA_KEY_DISPLAY_TITLE) ?: "", 256),
            artist = (meta?.getString(MediaMetadata.METADATA_KEY_ARTIST) ?: meta?.getString(MediaMetadata.METADATA_KEY_ALBUM_ARTIST)
                ?: meta?.getString(MediaMetadata.METADATA_KEY_DISPLAY_SUBTITLE))?.let { bounded(it, 256) },
            album = meta?.getString(MediaMetadata.METADATA_KEY_ALBUM)?.let { bounded(it, 256) },
            app = bounded(label, 64),
            durationMs = duration?.toULong(),
            positionMs = (duration?.let { position.coerceAtMost(it) } ?: position).toULong(),
            playing = playing,
            speedPermille = speed.toULong(),
            actions = actions(state.actions).toULong(),
        )
        publish(now, artOf(meta))
    }

    /** The cover, ≤ 256 px JPEG ≤ 96 KiB, compressed once per bitmap. */
    private fun artOf(meta: MediaMetadata?): ByteArray? {
        val bitmap = meta?.getBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART) ?: meta?.getBitmap(MediaMetadata.METADATA_KEY_ART)
            ?: meta?.getBitmap(MediaMetadata.METADATA_KEY_DISPLAY_ICON) ?: return null
        art?.let { (seen, bytes) -> if (seen === bitmap) return bytes }
        val bytes = try { compress(bitmap) } catch (_: Exception) { null }
        art = bitmap to bytes
        return bytes
    }

    /** A command the core already checked against the actions sent (§9.6, rule 4). */
    override fun command(action: String, positionMs: ULong?) {
        val c = controller ?: return
        val controls = c.transportControls
        when (action) {
            "play" -> controls.play()
            "pause" -> controls.pause()
            "toggle" -> if (c.playbackState?.state == PlaybackState.STATE_PLAYING) controls.pause() else controls.play()
            "next" -> controls.skipToNext()
            "previous" -> controls.skipToPrevious()
            "seek" -> positionMs?.let { controls.seekTo(it.toLong()) }
        }
    }

    companion object {
        /** The first PLAYING session, else the first in the system's order; -1 for none (§9.6, WN-46 review). */
        fun pick(states: List<Int?>): Int = states.indexOfFirst { it == PlaybackState.STATE_PLAYING }.takeIf { it >= 0 }
            ?: if (states.isEmpty()) -1 else 0

        /** Indexes of the sessions that aren't [own] (WN-47: our mirror of the PC's player is never sent back to the PC). */
        fun <T> eligible(sessions: List<T>, own: T?): List<Int> = sessions.indices.filter { own == null || sessions[it] != own }

        /** PlaybackState actions to §9.6 bits: 1 play, 2 pause, 4 next, 8 previous, 16 seek. */
        fun actions(flags: Long): Long {
            var bits = 0L
            if (flags and (PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PLAY_PAUSE) != 0L) bits = bits or 1
            if (flags and (PlaybackState.ACTION_PAUSE or PlaybackState.ACTION_PLAY_PAUSE) != 0L) bits = bits or 2
            if (flags and PlaybackState.ACTION_SKIP_TO_NEXT != 0L) bits = bits or 4
            if (flags and PlaybackState.ACTION_SKIP_TO_PREVIOUS != 0L) bits = bits or 8
            if (flags and PlaybackState.ACTION_SEEK_TO != 0L) bits = bits or 16
            return bits
        }

        /** At most [max] UTF-8 bytes, cut on a character boundary (the core refuses longer). */
        fun bounded(text: String, max: Int): String {
            if (text.toByteArray().size <= max) return text
            var end = text.length
            while (end > 0 && text.substring(0, end).toByteArray().size > max) end--
            if (end > 0 && Character.isHighSurrogate(text[end - 1])) end--
            return text.substring(0, end)
        }

        private fun compress(source: Bitmap): ByteArray? {
            val side = maxOf(source.width, source.height).coerceAtLeast(1)
            val scale = minOf(1f, 256f / side)
            val scaled = if (scale < 1f) Bitmap.createScaledBitmap(source, (source.width * scale).toInt().coerceAtLeast(1), (source.height * scale).toInt().coerceAtLeast(1), true) else source
            for (quality in intArrayOf(80, 60, 40)) {
                val out = ByteArrayOutputStream()
                scaled.compress(Bitmap.CompressFormat.JPEG, quality, out)
                if (out.size() <= 96 * 1024) return out.toByteArray()
            }
            return null
        }
    }
}

/**
 * CCS/1 RULE:exists only so Android lets WindConnect read media sessions (§9.6 rule 6): it reads no notifications. With the
 * setting off it unbinds itself so it doesn't keep the process resident (WN-46 review; a residency factor for WN-30).
 */
class MediaListenerService : NotificationListenerService() {
    override fun onListenerConnected() {
        current = WeakReference(this)
        if (!allowed(this)) requestUnbind()
    }
    override fun onListenerDisconnected() { current = null }

    companion object {
        @Volatile private var current: WeakReference<MediaListenerService>? = null

        fun granted(context: Context): Boolean =
            context.getSystemService(android.app.NotificationManager::class.java)
                .isNotificationListenerAccessGranted(ComponentName(context, MediaListenerService::class.java))

        private fun allowed(context: Context) = context.getSharedPreferences(WindCore.SETTINGS_FILE, Context.MODE_PRIVATE).getBoolean(WindCore.MEDIA_ALLOWED, true)

        /** Follows the setting: unbind when off, ask for a rebind when on. */
        fun follow(context: Context, allowed: Boolean) {
            if (!granted(context)) return
            if (allowed) requestRebind(ComponentName(context, MediaListenerService::class.java))
            else current?.get()?.requestUnbind()
        }
    }
}
