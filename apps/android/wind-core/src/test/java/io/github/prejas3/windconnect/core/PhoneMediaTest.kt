package io.github.prejas3.windconnect.core

import android.media.session.PlaybackState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class PhoneMediaTest {
    @Test fun thePlayingSessionWinsOverTheTopOne() {
        // A paused podcast app above a playing Spotify: Spotify (WN-46 review, point 3).
        assertEquals(1, PhoneMedia.pick(listOf(PlaybackState.STATE_PAUSED, PlaybackState.STATE_PLAYING)))
        assertEquals("none playing: the system's first", 0, PhoneMedia.pick(listOf(PlaybackState.STATE_PAUSED, PlaybackState.STATE_STOPPED)))
        assertEquals(0, PhoneMedia.pick(listOf(null)))
        assertEquals(-1, PhoneMedia.pick(emptyList()))
        assertEquals(2, PhoneMedia.pick(listOf(null, PlaybackState.STATE_BUFFERING, PlaybackState.STATE_PLAYING)))
    }

    @Test fun playbackActionsToProtocolBits() {
        assertEquals(1L or 2L, PhoneMedia.actions(PlaybackState.ACTION_PLAY_PAUSE))
        assertEquals(1L, PhoneMedia.actions(PlaybackState.ACTION_PLAY))
        assertEquals(31L, PhoneMedia.actions(PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PAUSE or PlaybackState.ACTION_SKIP_TO_NEXT
            or PlaybackState.ACTION_SKIP_TO_PREVIOUS or PlaybackState.ACTION_SEEK_TO))
        assertEquals(0L, PhoneMedia.actions(PlaybackState.ACTION_STOP))
    }

    @Test fun textsAreCutToTheProtocolsBytes() {
        assertEquals("abc", PhoneMedia.bounded("abc", 256))
        val polish = "ż".repeat(200) // 2 bytes each
        val cut = PhoneMedia.bounded(polish, 256)
        assertTrue(cut.toByteArray().size <= 256)
        assertEquals(128, cut.length)
        val emoji = "a" + "😀".repeat(100) // 4 bytes each, surrogate pairs
        val e = PhoneMedia.bounded(emoji, 10)
        assertTrue(e.toByteArray().size <= 10)
        assertTrue("no half surrogate", !Character.isHighSurrogate(e.last()))
    }
}
