package io.github.prejas3.windconnect

import android.graphics.Bitmap
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.background
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.core.ClipboardHistoryView
import io.github.prejas3.windconnect.ui.ThemeMode
import io.github.prejas3.windconnect.ui.WindConnectTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import java.io.ByteArrayOutputStream
import java.io.File

/** WN-117: the history card in Light and Dark: order as given, actions by id, preview shown as plain text. */
class ClipboardHistoryUiTest {
    @get:Rule val compose = createComposeRule()

    private fun thumbnail(): ByteArray {
        val bitmap = Bitmap.createBitmap(32, 20, Bitmap.Config.ARGB_8888).apply { eraseColor(0xFF3A7BD5.toInt()) }
        return ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
    }

    @Test fun historyCardInBothThemes() {
        val entries = listOf(
            ClipboardHistoryView(3, false, "Przypięty adres <b>nie HTML</b>", null, fromPc = true, pinned = true),
            ClipboardHistoryView(2, true, "Obraz 32×20", thumbnail(), fromPc = false, pinned = false),
            ClipboardHistoryView(1, false, "https://example.com/a", null, fromPc = false, pinned = false),
        )
        val calls = mutableListOf<String>()
        val actions = ClipboardHistoryActions({ calls += "copy $it" }, { id, pinned -> calls += "pin $id $pinned" }, { calls += "remove $it" }, { calls += "clear" })
        val theme = mutableStateOf(ThemeMode.Light)
        val list = mutableStateOf(entries)
        compose.setContent {
            WindConnectTheme(theme.value, false) {
                Column(Modifier.background(MaterialTheme.colorScheme.background).padding(16.dp).testTag("historyShot")) {
                    ClipboardHistoryCard(list.value, true, actions)
                }
            }
        }
        for (mode in listOf(ThemeMode.Light, ThemeMode.Dark)) {
            compose.runOnIdle { theme.value = mode }
            compose.onNodeWithText("Przypięty adres <b>nie HTML</b>").assertIsDisplayed()
            compose.onNodeWithText("Przypięte · z komputera").assertIsDisplayed()
            compose.onNodeWithText("Obraz 32×20").assertIsDisplayed()
            val shot = compose.onNodeWithTag("historyShot").captureToImage().asAndroidBitmap()
            File(InstrumentationRegistry.getInstrumentation().targetContext.cacheDir, "wn117-history-${mode.name}.png").outputStream().use {
                shot.compress(Bitmap.CompressFormat.PNG, 100, it)
            }
        }
        val order = compose.onAllNodes(hasTestTag("historyCopy3") or hasTestTag("historyCopy2") or hasTestTag("historyCopy1")).fetchSemanticsNodes()
            .map { it.config[androidx.compose.ui.semantics.SemanticsProperties.TestTag] }
        assertEquals(listOf("historyCopy3", "historyCopy2", "historyCopy1"), order)
        compose.onNodeWithTag("historyCopy2").performClick()
        // WN-160: pin and remove are icons on the entry's row, named for TalkBack.
        compose.onNodeWithTag("historyPin3").assertContentDescriptionEquals("Odepnij").performClick()
        compose.onNodeWithTag("historyPin1").assertContentDescriptionEquals("Przypnij").performClick()
        compose.onNodeWithTag("historyRemove1").assertContentDescriptionEquals("Usuń")
        compose.onNodeWithTag("historyRemove1").performClick()
        compose.onNodeWithTag("historyClear").performClick()
        compose.runOnIdle { assertEquals(listOf("copy 2", "pin 3 false", "pin 1 true", "remove 1", "clear"), calls) }

        // WN-160: an entry with nothing visible says so instead of an empty line; text has no monitor icon, same column as images.
        compose.runOnIdle { list.value = listOf(ClipboardHistoryView(9, false, "", null, fromPc = true, pinned = false)) + entries }
        compose.onNodeWithText("Pusty tekst (same spacje)").assertIsDisplayed()
        val textLeft = compose.onNodeWithText("https://example.com/a", useUnmergedTree = true).fetchSemanticsNode().boundsInRoot.left
        val imageLeft = compose.onNodeWithText("Obraz 32×20", useUnmergedTree = true).fetchSemanticsNode().boundsInRoot.left
        assertEquals("text and image previews start in one column", imageLeft, textLeft, 0.5f)
        // Only pins left: no Clear; empty: the explanation.
        compose.runOnIdle { list.value = entries.take(1) }
        compose.onNodeWithTag("historyClear").assertDoesNotExist()
        compose.runOnIdle { list.value = emptyList() }
        compose.onNodeWithTag("historyEmpty").assertIsDisplayed()
    }
}
