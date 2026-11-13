package io.github.prejas3.windconnect.core

import io.github.prejas3.windconnect.bindings.PhoneNotification
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** WN-45: the app filter, the quiet rule, and the send point's re-check. */
class NotifyAppsTest {
    private val signal = "org.thoughtcrime.securesms"
    private val bank = "pl.bank"

    @Test fun bothModesFilter() {
        assertTrue(AppFilter.allows(signal, NotifyAppMode.AllExcept, emptySet(), emptySet()))
        assertFalse(AppFilter.allows(signal, NotifyAppMode.AllExcept, setOf(signal), emptySet()))
        assertFalse("new apps are off", AppFilter.allows(signal, NotifyAppMode.OnlyChosen, emptySet(), emptySet()))
        assertTrue(AppFilter.allows(signal, NotifyAppMode.OnlyChosen, setOf(signal), setOf(signal)))
        // Each mode reads its own list.
        val state = NotifyAppsState(NotifyAppMode.AllExcept, blocked = setOf(bank), chosen = setOf(signal))
        assertTrue(state.allows(signal)); assertFalse(state.allows(bank))
        val only = state.copy(mode = NotifyAppMode.OnlyChosen)
        assertTrue(only.allows(signal)); assertFalse(only.allows("x"))
        assertFalse(only.sendsNothing)
        assertTrue(only.copy(chosen = emptySet()).sendsNothing)
        assertFalse(state.copy(blocked = setOf(signal, bank)).sendsNothing)
    }

    @Test fun quietOnlyWhenDoNotDisturbHoldsItBack() {
        assertTrue(NotificationRules.quiet(ranked = true, matchesInterruptionFilter = false))
        assertFalse(NotificationRules.quiet(ranked = true, matchesInterruptionFilter = true))
        assertFalse("no ranking: as before", NotificationRules.quiet(ranked = false, matchesInterruptionFilter = false))
    }

    private fun note(id: Int, pkg: String) = PhoneNotification(
        id = ByteArray(16) { id.toByte() }, app = "", packageName = pkg, title = "t", text = null,
        timeMs = 0uL, progress = false, reply = null, quiet = false,
    )

    @Test fun theSendPointChecksAgainAndBlockingRemovesWhatWasSent() = runBlocking {
        val allowed = mutableSetOf(signal, bank)
        val sent = mutableListOf<Int>()
        val removed = mutableListOf<Int>()
        val forgotten = mutableListOf<Int>()
        val forward = NotifyForwarder(
            allows = { it in allowed },
            send = { n, _ -> sent += n.id[0].toInt(); "sent" },
            removed = { removed += it[0].toInt() },
            forget = { ids -> forgotten += ids.map { it[0].toInt() } },
        )
        fun posted(id: Int, pkg: String) = NotificationEvent.Posted(note(id, pkg), pkg, 0)
        assertEquals("sent", forward.handle(posted(1, signal)))
        assertEquals("sent", forward.handle(posted(2, bank)))
        assertEquals("sent", forward.handle(posted(3, signal)))
        // Queued before the block, handled after it: dropped at the send point, icon and all (send is never called).
        val queued = posted(4, signal)
        allowed -= signal
        assertEquals("filtered", forward.handle(queued))
        assertEquals(listOf(1, 2, 3), sent)
        // The block: Removed for this session's signal ones, forgotten for reply/dismiss; the bank's stay.
        assertNull(forward.handle(NotificationEvent.FilterChanged))
        assertEquals(listOf(1, 3), removed)
        assertEquals(listOf(1, 3), forgotten)
        assertFalse(forward.isSent(ByteArray(16) { 1 }))
        assertTrue(forward.isSent(ByteArray(16) { 2 }))
        // Again: nothing more to remove.
        forward.handle(NotificationEvent.FilterChanged)
        assertEquals(listOf(1, 3), removed)
        // Gone on the phone: no longer sent; unblocking sends nothing retroactively.
        forward.handle(NotificationEvent.Removed(ByteArray(16) { 2 }))
        assertFalse(forward.isSent(ByteArray(16) { 2 }))
        allowed += signal
        forward.handle(NotificationEvent.FilterChanged)
        assertEquals(listOf(1, 2, 3), sent)
    }

    @Test fun onlySentOrHeldCount() = runBlocking {
        val forward = NotifyForwarder(allows = { true }, send = { _, _ -> "dropped" }, removed = {}, forget = {})
        forward.handle(NotificationEvent.Posted(note(5, signal), signal, 0))
        assertFalse(forward.isSent(ByteArray(16) { 5 }))
    }
}
