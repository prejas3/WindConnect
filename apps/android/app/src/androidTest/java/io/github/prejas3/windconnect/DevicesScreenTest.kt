package io.github.prejas3.windconnect

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import io.github.prejas3.windconnect.bindings.PairedEntry
import io.github.prejas3.windconnect.core.ConnectionState
import io.github.prejas3.windconnect.core.END_UNPAIRED_HERE
import io.github.prejas3.windconnect.core.REFUSED
import io.github.prejas3.windconnect.ui.ThemeMode
import io.github.prejas3.windconnect.ui.WindConnectTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test

/** WC-28: the paired-computer list, its inline rename and removal question, and the end messages. */
class DevicesScreenTest {
    @get:Rule val compose = createComposeRule()

    private val desk = PairedEntry(ByteArray(32) { 1 }, "Biurko", "aaaa1111bbbb2222", 1_794_300_000uL, false)
    private val laptop = PairedEntry(ByteArray(32) { 2 }, "Laptop", "cccc3333dddd4444", 1_794_390_000uL, false)
    private val gone = PairedEntry(ByteArray(32) { 3 }, "Komputer", "eeee5555ffff6666", 1_794_400_000uL, true)

    @Test fun listRenameAndUnpairStayInTheCard() {
        val calls = mutableListOf<String>()
        val state = PairingUi(ready = true, initialized = true, devices = listOf(desk, laptop, gone),
            connection = ConnectionState.Connected("Biurko", desk.fingerprint, 1000))
        compose.setContent { WindConnectTheme(ThemeMode.Light, false) {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                DevicesScreen(state, onBack = { calls += "back" }, onConnect = { calls += "connect ${it.name}" },
                    onRename = { e, n -> calls += "rename ${e.name} $n" }, onUnpair = { calls += "unpair ${it.name}" })
            }
        } }
        compose.onNodeWithText("Połączony", substring = true).assertExists()
        compose.onNodeWithText("komputer dowie się", substring = true).assertExists()
        // Connected computer: no "Connect"; the other one has it.
        compose.onAllNodesWithTag("deviceConnect").assertCountEquals(1)
        compose.onAllNodesWithTag("deviceConnect")[0].performScrollTo().performClick()
        assertEquals("connect Laptop", calls.last())

        // WN-170: rename and remove are in the ⋮ menu of each computer.
        compose.onAllNodesWithTag("deviceMenu")[1].performScrollTo().performClick()
        compose.onNodeWithTag("deviceRename").performClick()
        compose.onNodeWithTag("renameField").performTextReplacement("   ")
        compose.onNodeWithTag("renameSave").performClick()
        compose.onNodeWithText("Nazwa nie może być pusta").assertExists()
        compose.onNodeWithTag("renameField").performTextReplacement("  Laptop Oli ")
        compose.onNodeWithTag("renameSave").performClick()
        assertEquals("rename Laptop Laptop Oli", calls.last())

        compose.onAllNodesWithTag("deviceMenu")[0].performScrollTo().performClick()
        compose.onNodeWithTag("deviceUnpair").performClick()
        compose.onNodeWithTag("unpairQuestion").assertTextContains("Komputer też je usunie.", substring = true)
        compose.onNodeWithText("Anuluj").performClick()
        assert(calls.none { it.startsWith("unpair") }) { "Anuluj removes nothing: $calls" }
        compose.onAllNodesWithTag("deviceMenu")[1].performScrollTo().performClick()
        compose.onNodeWithTag("deviceUnpair").performClick()
        compose.onNodeWithTag("unpairQuestion").assertTextContains("gdy znajdzie się w tej samej sieci", substring = true)
        compose.onNodeWithTag("unpairConfirm").performClick()
        assertEquals("unpair Laptop", calls.last())
    }

    @Test fun endMessagesForUnpairBusyAndRefusal() {
        assertEquals("Komputer usunął parowanie z tym telefonem", endMessage(ConnectionState.Disconnected("unpaired")))
        assertEquals("Usunięto parowanie z komputerem", endMessage(ConnectionState.Disconnected(END_UNPAIRED_HERE)))
        assert(endMessage(ConnectionState.Disconnected("busy"))!!.startsWith("Komputer jest połączony z innym telefonem"))
        assertEquals("Komputer odrzuca połączenie — sparuj ponownie", endMessage(ConnectionState.Disconnected("untrusted")))
        assertEquals(R.string.end_refused, endMessageRes(ConnectionState.Failed(REFUSED)))
        assertNull(endMessage(ConnectionState.Disconnected("going-away")))
    }
}
