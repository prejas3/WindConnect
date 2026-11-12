package io.github.prejas3.windconnect

import android.graphics.Bitmap
import android.graphics.Color
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import java.util.concurrent.atomic.AtomicInteger

/**
 * CCS/1 PURPOSE:WN-46 phone side of check-media-android.cjs. A test MediaSession stands in for Spotify (not on the emulator):
 * title, artist, 512 px art, PLAYING. Each command changes its state the way a player would (toggle, next = a new title, seek),
 * so the PC sees the result. After 10 toggles, 10 nexts and a seek it drops NEXT from its actions and checks that the PC's
 * further "next" gets no reaction (PROTOCOL.md §9.6 rule 4).
 */
object MediaProof {
    fun run() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val thread = HandlerThread("media proof").also { it.start() }
        val handler = Handler(thread.looper)
        val session = MediaSession(context, "WindConnect media proof")
        val all = PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PAUSE or PlaybackState.ACTION_PLAY_PAUSE or
            PlaybackState.ACTION_SKIP_TO_NEXT or PlaybackState.ACTION_SKIP_TO_PREVIOUS or PlaybackState.ACTION_SEEK_TO
        var actions = all
        var playing = true
        var position = 30_000L
        var track = 1
        val toggles = AtomicInteger(); val nexts = AtomicInteger(); val seeks = AtomicInteger(); val previous = AtomicInteger()
        fun state() = session.setPlaybackState(PlaybackState.Builder()
            .setState(if (playing) PlaybackState.STATE_PLAYING else PlaybackState.STATE_PAUSED, position, 1f, SystemClock.elapsedRealtime())
            .setActions(actions).build())
        fun metadata() {
            val art = Bitmap.createBitmap(512, 512, Bitmap.Config.ARGB_8888).apply { eraseColor(if (track % 2 == 1) Color.rgb(240, 163, 94) else Color.rgb(67, 201, 184)) }
            session.setMetadata(MediaMetadata.Builder()
                .putString(MediaMetadata.METADATA_KEY_TITLE, "Proof song $track")
                .putString(MediaMetadata.METADATA_KEY_ARTIST, "WindConnect test")
                .putLong(MediaMetadata.METADATA_KEY_DURATION, 200_000)
                .putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, art)
                .build())
        }
        session.setCallback(object : MediaSession.Callback() {
            override fun onPlay() { toggles.incrementAndGet(); playing = true; state() }
            override fun onPause() { toggles.incrementAndGet(); playing = false; state() }
            override fun onSkipToNext() { nexts.incrementAndGet(); track++; position = 0; metadata(); state() }
            override fun onSkipToPrevious() { previous.incrementAndGet(); track = maxOf(1, track - 1); position = 0; metadata(); state() }
            override fun onSeekTo(pos: Long) { seeks.incrementAndGet(); position = pos; state() }
        }, handler)
        handler.post { metadata(); state(); session.isActive = true }
        fun waitFor(label: String, limitMs: Long = 120_000, done: () -> Boolean) {
            val deadline = SystemClock.elapsedRealtime() + limitMs
            while (!done()) { assertTrue(label, SystemClock.elapsedRealtime() < deadline); Thread.sleep(20) }
        }
        try {
            waitFor("10 toggles, 10 nexts, a seek from the PC") { toggles.get() >= 10 && nexts.get() >= 10 && seeks.get() >= 1 }
            // Next no longer allowed: the PC's "next" must not reach the player.
            val before = nexts.get()
            handler.post { actions = all and PlaybackState.ACTION_SKIP_TO_NEXT.inv(); state() }
            Thread.sleep(6_000)
            assertEquals("a command outside `actions` gets no reaction", before, nexts.get())
            instrumentation.sendStatus(0, Bundle().apply {
                putString("mediaProof", "toggles=${toggles.get()} nexts=${nexts.get()} seeks=${seeks.get()} previous=${previous.get()} disallowedIgnored=true")
            })
        } finally {
            session.release()
            thread.quitSafely()
        }
    }
}
