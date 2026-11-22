package io.github.prejas3.windconnect

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import com.google.android.apps.common.testing.accessibility.framework.integrations.espresso.AccessibilityValidator
import io.github.prejas3.windconnect.bindings.PairedEntry
import io.github.prejas3.windconnect.core.ConnectionState
import io.github.prejas3.windconnect.ui.ThemeMode
import io.github.prejas3.windconnect.ui.WindConnectTheme
import org.junit.Before
import org.junit.Rule
import org.junit.Test

/**
 * WN-64: TalkBack and large text. Google's Accessibility Test Framework checks each screen part by part while it
 * scrolls (labels, touch targets of 48 dp, text and image contrast, duplicate descriptions, clickable spans) in both
 * themes and at font scale 1 and 2 (Android's largest "Font size" with non-linear scaling stays below 2).
 */
class AccessibilityTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    @Before fun checks() {
        val atf = AccessibilityValidator().setRunChecksFromRootView(true)
        compose.setComposeAccessibilityValidator(object : androidx.compose.ui.test.ComposeAccessibilityValidator { override fun check(view: android.view.View) { atf.check(view) } })
    }

    private val desk = PairedEntry(ByteArray(32) { 1 }, "Biurko", "aaaa1111bbbb2222", 1_794_300_000uL, false)
    private val laptop = PairedEntry(ByteArray(32) { 2 }, "Laptop", "cccc3333dddd4444", 1_794_390_000uL, false)
    private val connected = PairingUi(ready = true, initialized = true, hasPairedDevice = true, filesAccepted = true,
        computerName = "Biurko", devices = listOf(desk, laptop), connection = ConnectionState.Connected("Biurko", desk.fingerprint, 4200))

    private val case = androidx.compose.runtime.mutableStateOf(ThemeMode.Light to 1f)

    private fun screen(content: @Composable () -> Unit) {
        compose.setContent {
            val (theme, scale) = case.value
            val base = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(base.density, scale)) {
                WindConnectTheme(theme, false) {
                    // A new scroll state per case: each case starts at the top.
                    androidx.compose.runtime.key(case.value) {
                        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(18.dp).testTag("a11yScroll")) { content() }
                    }
                }
            }
        }
    }

    /** Checks what's on screen, then scrolls on until the end, checking each part. */
    private fun checkWhileScrolling() {
        repeat(30) {
            compose.onRoot().tryPerformAccessibilityChecks()
            val before = compose.onNodeWithTag("a11yScroll").fetchSemanticsNode().config[androidx.compose.ui.semantics.SemanticsProperties.VerticalScrollAxisRange].value()
            compose.onNodeWithTag("a11yScroll").performTouchInput { swipeUp() }
            compose.waitForIdle()
            val after = compose.onNodeWithTag("a11yScroll").fetchSemanticsNode().config[androidx.compose.ui.semantics.SemanticsProperties.VerticalScrollAxisRange].value()
            if (after == before) return
        }
    }

    private fun everyCase(content: @Composable () -> Unit) {
        // Light and dark themes, normal and doubled text; each a fresh composition.
        screen(content)
        for (next in listOf(ThemeMode.Light to 1f, ThemeMode.Dark to 1f, ThemeMode.Light to 2f)) {
            compose.runOnIdle { case.value = next }
            compose.waitForIdle()
            checkWhileScrolling()
        }
    }

    @Test fun homeUnpaired() = everyCase { HomeScreen(PairingUi(ready = true, initialized = true), {}, {}, onNearby = {}) }

    @Test fun homeConnected() = everyCase {
        HomeScreen(connected, {}, {}, onClipboard = {}, onClipboardEnabled = {}, onSendFile = {})
        ClipboardSettingRows(connected, {}, {})
    }

    @Test fun pairedComputers() = everyCase { DevicesScreen(connected, onBack = {}, onConnect = {}, onRename = { _, _ -> }, onUnpair = {}) }

    @Test fun settings() {
        // WN-130: the settings screen's groups, as PairingScreen lays them out.
        everyCase {
            io.github.prejas3.windconnect.ui.WindSection("Phone on the computer") { NotifyControls(); SmsControls(); CallControls(); GalleryControls(); MediaControls() }
            io.github.prejas3.windconnect.ui.WindSection("Computer on the phone") { PcMediaControls(); LinkControls(); FindControls(); ReceiveSettingRow() }
            io.github.prejas3.windconnect.ui.WindSection("Clipboard, shortcuts and background") { BackgroundControls(true); ClipboardTileControls(); LauncherShortcutControls(); WidgetControls(); RelayControls() }
        }
        // TalkBack's heading navigation jumps between the settings groups.
        val headings = compose.onAllNodes(isHeading(), useUnmergedTree = true).fetchSemanticsNodes().size
        org.junit.Assert.assertTrue("headings: $headings", headings >= 3)
    }

    /** WN-156 (WN-64 touch targets): the own slider, a segment and a row's "More" are at least 48 dp tall. */
    @Test fun ownControlsAreLargeEnough() {
        screen {
            io.github.prejas3.windconnect.ui.WindSlider(0.4f, {}, Modifier.testTag("slider"))
            io.github.prejas3.windconnect.ui.WindSegmented(listOf(5, 16), 5, { "$it MB" }, {}, tag = { "seg$it" })
            io.github.prejas3.windconnect.ui.WindSettingRow("pc", "Row", "Description", more = { androidx.compose.material3.Text("Details") })
        }
        compose.onNodeWithTag("slider").assertHeightIsAtLeast(48.dp)
        compose.onNodeWithTag("seg16").assertHeightIsAtLeast(48.dp)
        compose.onNodeWithContentDescription("More: Row").assertHeightIsAtLeast(48.dp)
    }

    @Test fun pairedAndReceiving() = everyCase { DoneScreen("Biurko", "Connected to the computer", onDone = {}); ReceiveFilesControls() }

    /** The harness itself: an unlabeled 10 dp button must fail the checks, or the passes above prove nothing. */
    @Test fun checksCatchABadControl() {
        screen {
            androidx.compose.foundation.layout.Box(Modifier.padding(8.dp).size(10.dp)
                .clickable(onClick = {}).testTag("bad"))
        }
        val failed = runCatching { compose.onRoot().tryPerformAccessibilityChecks() }.isFailure
        org.junit.Assert.assertTrue("ATF found no problem with an unlabeled 10 dp button", failed)
    }
}
