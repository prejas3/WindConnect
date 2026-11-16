package io.github.prejas3.windconnect.core

import org.junit.Assert.*
import org.junit.Test

/** WN-117: the history's rules (as clipboard_history.rs on the PC). */
class ClipboardHistoryTest {
    private fun text(value: String, fromPc: Boolean = false) = ClipboardHistoryItem.Text(value, fromPc)
    private fun image(seed: Int, bytes: Int) = ClipboardHistoryItem.Image(ByteArray(bytes) { seed.toByte() }, ByteArray(0), 1, 1, false)

    @Test fun newestFirstAndRepeatMovesUp() {
        val history = ClipboardHistory()
        history.record(text("a")); history.record(text("b")); history.record(text("a"))
        assertEquals(listOf("a", "b"), history.views().map { it.preview })
    }

    @Test fun twentyRecentAndPinnedOnTopOutsideThem() {
        val history = ClipboardHistory()
        val first = history.record(text("0"))
        assertEquals(PinResult.Ok, history.pin(first, true))
        for (i in 1..25) history.record(text("$i"))
        val views = history.views()
        assertEquals(21, views.size)
        assertEquals("0", views[0].preview); assertTrue(views[0].pinned)
        assertEquals((25 downTo 6).map { "$it" }, views.drop(1).map { it.preview })
        // A pinned item copied again stays one entry, pinned.
        history.record(text("0"))
        assertEquals(21, history.views().size)
    }

    @Test fun atMostTwentyPins() {
        val history = ClipboardHistory()
        repeat(20) { assertEquals(PinResult.Ok, history.pin(history.record(text("$it")), true)) }
        assertEquals(PinResult.Full, history.pin(history.record(text("21")), true))
        assertEquals(21, history.views().size)
        assertEquals(PinResult.Missing, history.pin(999, true))
    }

    @Test fun imageBudgetDropsOldestImageAndLimitsPins() {
        val history = ClipboardHistory()
        val old = history.record(image(1, 30_000_000))
        history.record(text("t"))
        history.record(image(2, 30_000_000))
        val newest = history.record(image(3, 30_000_000))
        assertNull(history.get(old))
        assertEquals(3, history.views().size)
        assertEquals(PinResult.Ok, history.pin(newest, true))
        val other = history.views().first { it.image && !it.pinned }.id
        assertEquals(PinResult.Ok, history.pin(other, true))
        val third = history.record(image(4, 10_000_000))
        assertEquals(PinResult.TooLarge, history.pin(third, true))
    }

    @Test fun clearKeepsPinsAndRemoveSaysIfPinned() {
        val history = ClipboardHistory()
        val pinned = history.record(text("p")); history.pin(pinned, true)
        val plain = history.record(text("q"))
        history.clear()
        assertEquals(listOf("p"), history.views().map { it.preview })
        assertNull(history.remove(plain))
        assertEquals(true, history.remove(pinned))
        assertFalse(history.hasPins)
    }

    @Test fun pinsRoundTripAndBrokenDataLoadsNothing() {
        val history = ClipboardHistory()
        val a = history.record(text("from pc", fromPc = true)); history.pin(a, true)
        val b = history.record(ClipboardHistoryItem.Image(byteArrayOf(1, 2, 3), byteArrayOf(9), 4, 5, true)); history.pin(b, true)
        history.record(text("not pinned"))
        val plain = history.encodePins()
        val loaded = ClipboardHistory()
        assertTrue(loaded.loadPins(plain))
        val views = loaded.views()
        assertEquals(listOf("Obraz 4×5", "from pc"), views.map { it.preview })
        assertTrue(views.all { it.pinned && it.fromPc })
        val png = (loaded.get(views[0].id) as ClipboardHistoryItem.Image)
        assertArrayEquals(byteArrayOf(1, 2, 3), png.png); assertArrayEquals(byteArrayOf(9), png.thumbnail)
        val broken = ClipboardHistory()
        assertFalse(broken.loadPins(plain.copyOf(plain.size - 1)))
        assertFalse(broken.loadPins(plain + byteArrayOf(0)))
        assertTrue(broken.views().isEmpty())
    }

    @Test fun previewIsOneSafeLine() {
        assertEquals("a b c", ClipboardHistory.preview("a\nb‮c"))
        assertEquals("x y", ClipboardHistory.preview("x​y"))
        val long = ClipboardHistory.preview("z".repeat(500))
        assertEquals(121, long.length); assertTrue(long.endsWith("…"))
    }
}
