package io.github.prejas3.windconnect.core

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.PersistableBundle
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * CCS/1 PURPOSE:WN-117 on a real clipboard and Keystore: order, the twenty, pins kept encrypted across a restart, flags kept
 * on copy back, sync off keeps nothing, and a clip marked IS_SENSITIVE never reaches the history.
 */
@RunWith(AndroidJUnit4::class)
class ClipboardHistoryInstrumentedTest {
    private val context: Context = InstrumentationRegistry.getInstrumentation().targetContext
    private val manager = context.getSystemService(ClipboardManager::class.java)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val pins = File(context.cacheDir, "wn117-pins.bin")
    private val prefs = context.getSharedPreferences("wn117-test", Context.MODE_PRIVATE)
    private lateinit var activity: ActivityScenario<Activity>
    private val sent = mutableListOf<String>()

    @Before fun setUp() {
        pins.delete(); prefs.edit().clear().commit()
        activity = ActivityScenario.launch(Activity::class.java)
        val deadline = System.currentTimeMillis() + 5_000
        var focused = false
        while (!focused && System.currentTimeMillis() < deadline) {
            activity.onActivity { focused = it.hasWindowFocus() }
            if (!focused) Thread.sleep(50)
        }
        assertTrue("the test Activity needs focus to read the clipboard", focused)
    }
    @After fun tearDown() { activity.close(); scope.cancel(); pins.delete() }

    private fun clipboard() = SystemClipboard(context, prefs, scope, {}, { true }, pinStore = ClipboardPinStore(context, pins)) { text, _, _ ->
        synchronized(sent) { sent += text }; "applied"
    }
    private fun copy(text: String, sensitive: Boolean = false) = InstrumentationRegistry.getInstrumentation().runOnMainSync {
        manager.setPrimaryClip(ClipData.newPlainText("WN-117 test", text).apply {
            if (sensitive) description.extras = PersistableBundle().apply { putBoolean(SystemClipboard.SENSITIVE, true) }
        })
    }
    private fun previews(clipboard: SystemClipboard) = clipboard.historyUi.value.map { it.preview }

    @Test fun historyFollowsTheRules() = runBlocking {
        val clipboard = clipboard()
        // A password manager's clip: neither sent nor kept.
        copy("WN117-SECRET", sensitive = true)
        assertEquals("sensitive", clipboard.send(true))
        assertTrue(previews(clipboard).isEmpty())
        assertFalse(sent.contains("WN117-SECRET"))

        for (i in 1..22) { copy("phone $i"); assertEquals("applied", clipboard.send(true)) }
        assertEquals((22 downTo 3).map { "phone $it" }, previews(clipboard))
        assertTrue(clipboard.historyUi.value.none { it.fromPc })

        assertTrue(clipboard.applyText("from pc"))
        val fromPc = clipboard.historyUi.value.first()
        assertEquals("from pc", fromPc.preview); assertTrue(fromPc.fromPc)
        assertEquals(20, clipboard.historyUi.value.size)

        // Copy back keeps the flags: from the PC sensitive as at receipt (so not sent again), from the phone plain.
        copy("other")
        assertTrue(clipboard.historyCopy(fromPc.id))
        assertClip("from pc", sensitive = true)
        assertEquals("sensitive", clipboard.send(true))
        val phone = clipboard.historyUi.value.first { it.preview == "phone 10" }
        assertTrue(clipboard.historyCopy(phone.id))
        assertClip("phone 10", sensitive = false)
        // Sent again, it moves to the top instead of repeating.
        assertEquals("applied", clipboard.send(true))
        assertEquals("phone 10", previews(clipboard).first()); assertEquals(1, previews(clipboard).count { it == "phone 10" })

        // Pins: on top, encrypted on disk, back after a restart.
        assertEquals(PinResult.Ok, clipboard.historyPin(fromPc.id, true))
        assertEquals("from pc", previews(clipboard).first())
        awaitFile()
        val raw = pins.readBytes()
        assertEquals("WCPN", String(raw.copyOfRange(0, 4), Charsets.US_ASCII))
        assertFalse("pin file holds no plain text", String(raw, Charsets.ISO_8859_1).contains("from pc"))
        clipboard.close()
        val restarted = clipboard()
        withTimeout(5_000) { restarted.historyUi.first { it.isNotEmpty() } }
        assertEquals(listOf("from pc"), previews(restarted))
        assertTrue(restarted.historyUi.value.single().pinned && restarted.historyUi.value.single().fromPc)

        // Sync off: unpinned ones go, nothing new comes in (received or sent by hand).
        copy("before off"); assertEquals("applied", restarted.send(true))
        restarted.setEnabled(false)
        assertEquals(listOf("from pc"), previews(restarted))
        assertFalse(restarted.applyText("while off"))
        copy("manual while off"); restarted.send(true)
        assertEquals(listOf("from pc"), previews(restarted))

        // Unpinning the last pin removes the file.
        restarted.setEnabled(true)
        restarted.historyRemove(restarted.historyUi.value.single().id)
        withTimeout(5_000) { while (pins.exists()) delay(20) }
        restarted.close()
    }

    private suspend fun awaitFile() = withTimeout(5_000) { while (!pins.exists()) delay(20) }
    private fun assertClip(text: String, sensitive: Boolean) = InstrumentationRegistry.getInstrumentation().runOnMainSync {
        val clip = manager.primaryClip!!
        assertEquals(text, clip.getItemAt(0).text.toString())
        assertEquals(sensitive, clip.description.extras?.getBoolean(SystemClipboard.SENSITIVE, false) ?: false)
    }
}
