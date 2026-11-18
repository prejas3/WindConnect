package io.github.prejas3.windconnect

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.bindings.FileTransfer
import io.github.prejas3.windconnect.core.ReceivedFile
import io.github.prejas3.windconnect.ui.*
import org.junit.Rule
import org.junit.Test
import org.junit.Assert.*
import java.io.File

class ReceiveFilesUiTest {
    @get:Rule val compose = createComposeRule()
    @Test fun lightAt150() = check(ThemeMode.Light)
    @Test fun darkAt150() = check(ThemeMode.Dark)
    private fun check(mode: ThemeMode) {
        val state = ReceiveUi(ready = true, transfers = listOf(FileTransfer("ab".repeat(32), true, "01".repeat(16), "folder/photo.jpg", 1024uL, 512uL, "running", null)),
            receipts = listOf(ReceivedFile("receipt", "data (1).json", "Download/WindConnect/folder/", 131079, "content://media/external_primary/downloads/1")))
        compose.setContent { WindConnectTheme(mode, false) {
            CompositionLocalProvider(LocalDensity provides Density(LocalDensity.current.density, 1.5f)) {
                Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(20.dp)) { ReceiveFilesControls(state, compact = false) }
            }
        } }
        compose.onNodeWithText("Gotowy · Pobrane/WindConnect").assertExists()
        compose.onNodeWithText("Anuluj odbiór").performScrollTo().assertIsEnabled()
        val progress = compose.onNode(SemanticsMatcher.keyIsDefined(SemanticsProperties.ProgressBarRangeInfo)).fetchSemanticsNode()
        assertEquals(.5f, progress.config[SemanticsProperties.ProgressBarRangeInfo].current)
        val image = compose.onRoot().captureToImage().asAndroidBitmap()
        File(InstrumentationRegistry.getInstrumentation().targetContext.cacheDir, "wn120-${mode.name}-150.png").outputStream().use { image.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }; image.recycle()
        compose.onNodeWithText("Otwórz Pobrane").performScrollTo().assertIsEnabled()
        compose.onNodeWithTag("received-receipt").performScrollTo().assertIsEnabled().assertTextContains("data (1).json", substring = true)
    }
}
