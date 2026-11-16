package io.github.prejas3.windconnect

import android.view.KeyEvent
import io.github.prejas3.windconnect.bindings.TouchpadCommand
import org.junit.Assert.*
import org.junit.Test

class PresenterTest {
    @Test fun volumeKeysMapToArrowSlideKeysOnly() {
        assertEquals(39L, PresenterKeys.slide(KeyEvent.KEYCODE_VOLUME_UP))
        assertEquals(37L, PresenterKeys.slide(KeyEvent.KEYCODE_VOLUME_DOWN))
        assertNull(PresenterKeys.slide(KeyEvent.KEYCODE_VOLUME_MUTE))
        assertNull(PresenterKeys.slide(KeyEvent.KEYCODE_BACK))
        assertEquals(TouchpadCommand.Key(39uL, 0u), PresenterKeys.command(PresenterKeys.NEXT))
    }

    @Test fun volumeKeysStayTheSystemsUnlessPresenterIsShownAndLive() {
        val up = KeyEvent.KEYCODE_VOLUME_UP
        assertEquals(39L, PresenterKeys.gate(up, presenting = true, live = true, keyboard = true))
        for ((presenting, live, keyboard) in listOf(Triple(false, true, true), Triple(true, false, true), Triple(true, true, false)))
            assertNull(PresenterKeys.gate(up, presenting, live, keyboard))
        assertNull(PresenterKeys.gate(KeyEvent.KEYCODE_ENTER, presenting = true, live = true, keyboard = true))
    }

    @Test fun gyroTurnsIntoPointerUnitsWithDeadZoneAndGapGuard() {
        // Turning left (positive yaw) moves left; tilting up (positive pitch) moves up.
        val (dx, dy) = LaserMotion.delta(pitchRate = 1f, yawRate = 1f, dtNanos = 10_000_000)
        assertEquals(-1100L, dx); assertEquals(-1100L, dy)
        assertEquals(0L to 0L, LaserMotion.delta(0.01f, -0.01f, 10_000_000))
        assertEquals(0L to 0L, LaserMotion.delta(1f, 1f, 0))
        assertEquals(0L to 0L, LaserMotion.delta(1f, 1f, 500_000_000))
    }

    @Test fun timerStartsOnFirstSlidePausesAndResets() {
        var now = 1_000L
        val timer = PresentationTimer { now }
        assertEquals(0L, timer.elapsed())
        timer.slideChanged(); now += 65_000
        assertEquals("01:05", PresentationTimer.format(timer.elapsed()))
        timer.pause(); now += 10_000
        assertEquals(65_000L, timer.elapsed())
        timer.slideChanged() // already used: a paused talk stays paused
        assertFalse(timer.running)
        timer.start(); now += 5_000
        assertEquals(70_000L, timer.elapsed())
        timer.reset()
        assertEquals(0L, timer.elapsed()); assertFalse(timer.used)
        assertEquals("1:01:01", PresentationTimer.format(3_661_000))
    }
}
