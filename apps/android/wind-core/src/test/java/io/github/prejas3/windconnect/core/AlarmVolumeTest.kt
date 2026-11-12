package io.github.prejas3.windconnect.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AlarmVolumeTest {
    private class Memory : AlarmVolume.Store {
        val values = HashMap<String, Int>()
        override fun get(key: String) = values[key]
        override fun put(key: String, value: Int) { values[key] = value }
        override fun remove(key: String) { values.remove(key) }
    }
    private class Stream(override var current: Int, override val max: Int = 7) : AlarmVolume.Volume {
        override fun set(value: Int) { current = value }
    }

    @Test fun ringRaisesThenRestores() {
        val (store, stream) = Memory() to Stream(2)
        val volume = AlarmVolume(store, stream)
        volume.raise()
        assertEquals(7, stream.current)
        volume.restore()
        assertEquals(2, stream.current)
        assertTrue("nothing left to repair", store.values.isEmpty())
    }

    @Test fun processDeathIsRepairedOnTheNextStart() {
        val store = Memory()
        AlarmVolume(store, Stream(3)).let { it.raise() }
        // The process died while ringing: a new process finds the stream still at our max.
        val after = Stream(7)
        AlarmVolume(store, after).restore()
        assertEquals(3, after.current)
        assertNull(store.values[AlarmVolume.PREVIOUS])
    }

    @Test fun aVolumeThePersonChangedStays() {
        val store = Memory()
        AlarmVolume(store, Stream(3)).raise()
        val changed = Stream(5)
        AlarmVolume(store, changed).restore()
        assertEquals("theirs, not ours", 5, changed.current)
        assertTrue(store.values.isEmpty())
    }

    @Test fun aSecondRaiseKeepsTheOriginalValue() {
        val (store, stream) = Memory() to Stream(1)
        val volume = AlarmVolume(store, stream)
        volume.raise()
        volume.raise()
        volume.restore()
        assertEquals(1, stream.current)
    }

    @Test fun restoreWithoutRaiseDoesNothing() {
        val stream = Stream(4)
        AlarmVolume(Memory(), stream).restore()
        assertEquals(4, stream.current)
    }

    @Test fun toneIsAudibleAndLoopable() {
        val tone = RingTone.pcm()
        assertEquals(44_100 * 12 / 10, tone.size)
        assertTrue("loud beeps", tone.maxOf { kotlin.math.abs(it.toInt()) } > 20_000)
        assertEquals("starts at zero (no click at the loop point)", 0, tone.first().toInt())
        assertEquals("ends silent", 0, tone.last().toInt())
    }
}
