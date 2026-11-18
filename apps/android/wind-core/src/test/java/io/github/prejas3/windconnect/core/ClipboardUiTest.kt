package io.github.prejas3.windconnect.core

import org.junit.Assert.*
import org.junit.Test

/** WN-153: the home screen's clipboard line hides the switch's own status by a coded flag, never by its text. */
class ClipboardUiTest {
    @Test fun aToggleLeavesNoLineAndAnEventShowsOne() {
        val toggled = ClipboardUi().withSwitch(false, "Synchronizacja wyłączona")
        assertFalse(toggled.enabled)
        assertNull(toggled.eventStatus)
        val received = toggled.withSwitch(true, "Synchronizacja automatyczna").withEvent("Odebrano schowek")
        assertEquals("Odebrano schowek", received.eventStatus)
        // Text from another language changes nothing: only the flag counts.
        assertNull(received.withSwitch(true, "Odebrano schowek").eventStatus)
    }
    @Test fun noStatusNoLine() {
        assertNull(ClipboardUi().eventStatus)
        assertNull(ClipboardUi().withEvent("").eventStatus)
    }
}
