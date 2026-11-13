package io.github.prejas3.windconnect.core

import android.media.session.PlaybackState
import org.junit.Assert.assertEquals
import org.junit.Test

class PcPlayerTest {
    @Test fun ourOwnSessionIsNeverSentBackToThePc() {
        // WN-47 review, point 1: the mirrored "WindConnect · PC" session would echo PC -> phone -> PC.
        // Tokens stand in as strings: our mirror is left out, every other session (also in this process) counts.
        val own = "our-pc-mirror"
        assertEquals(listOf(0, 2), PhoneMedia.eligible(listOf("spotify", own, "test-player-in-process"), own))
        assertEquals(emptyList<Int>(), PhoneMedia.eligible(listOf(own), own))
        assertEquals("no mirror yet: all count", listOf(0, 1), PhoneMedia.eligible(listOf("a", "b"), null))
    }

    @Test fun textsFromThePcAreOneCleanLine() {
        assertEquals("Ka onashi x", PcPlayer.clean("Ka" + Char(0x202E) + "onashi\nx"))
        assertEquals("", PcPlayer.clean("" + Char(0x200B) + Char(0x200B)))
        assertEquals(256, PcPlayer.clean("y".repeat(300)).length)
    }

    @Test fun protocolBitsBackToPlaybackActions() {
        assertEquals(PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PAUSE or PlaybackState.ACTION_PLAY_PAUSE, PcPlayer.playbackActions(3))
        assertEquals(PlaybackState.ACTION_PLAY, PcPlayer.playbackActions(1))
        assertEquals(PlaybackState.ACTION_SKIP_TO_NEXT or PlaybackState.ACTION_SEEK_TO, PcPlayer.playbackActions(4 or 16))
    }

    @Test fun progressExtrapolatesWhilePlaying() {
        val m = PcMedia("t", null, null, "Brave", 200_000, 30_000, 1_000, true, 1000, 31, null)
        assertEquals(40_000, m.positionAt(11_000))
        assertEquals(30_000, m.copy(playing = false).positionAt(11_000))
        assertEquals(200_000, m.positionAt(1_000_000))
    }
}
