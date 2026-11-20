package io.github.prejas3.windconnect

import android.graphics.Bitmap
import android.os.ParcelFileDescriptor
import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.unit.dp
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.core.ClipboardHistory
import io.github.prejas3.windconnect.core.ClipboardHistoryView
import io.github.prejas3.windconnect.core.ClipboardUi
import io.github.prejas3.windconnect.core.ConnectionState
import io.github.prejas3.windconnect.core.NotifyAppMode
import io.github.prejas3.windconnect.core.NotifyAppsState
import io.github.prejas3.windconnect.ui.*
import kotlinx.coroutines.runBlocking
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import java.io.ByteArrayOutputStream

/**
 * WN-160: screenshots of the home screen, clipboard history, settings and widget at a S25 Ultra's width (412 dp),
 * in the app's own dark theme and Material You. Not a check: run on demand with -e uiShots true; PNGs go to the app's
 * cache (shots/), pulled with adb.
 */
class UiShotsTest {
    @get:Rule val compose = createAndroidComposeRule<androidx.activity.ComponentActivity>()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val tag = InstrumentationRegistry.getArguments().getString("shotTag") ?: "shot"

    /** To /data/local/tmp/wn160 through the shell: the app (and its cache) is uninstalled after the run. */
    private fun save(name: String, bitmap: Bitmap) {
        shell("mkdir -p /data/local/tmp/wn160")
        val bytes = ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
        val pipes = instrumentation.uiAutomation.executeShellCommandRw("dd of=/data/local/tmp/wn160/$tag-$name.png")
        ParcelFileDescriptor.AutoCloseOutputStream(pipes[1]).use { it.write(bytes) }
        ParcelFileDescriptor.AutoCloseInputStream(pipes[0]).use { it.readBytes() }
    }

    private fun shell(command: String): String = ParcelFileDescriptor.AutoCloseInputStream(
        instrumentation.uiAutomation.executeShellCommand(command)).use { String(it.readBytes()) }

    private fun texts(view: android.view.View): List<android.widget.TextView> = when (view) {
        is android.widget.TextView -> listOf(view)
        is android.view.ViewGroup -> (0 until view.childCount).flatMap { texts(view.getChildAt(it)) }
        else -> emptyList()
    }

    private fun enabled() = assumeTrue("screenshots only on demand", InstrumentationRegistry.getArguments().getString("uiShots") == "true")

    private fun image(color: Int, w: Int, h: Int): ByteArray {
        val bitmap = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888).apply { eraseColor(color) }
        return ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
    }

    private val history = listOf(
        ClipboardHistoryView(6, false, ClipboardHistory.preview("\n\n\n   \n" + " ".repeat(200) + "tekst po pustych liniach"), null, fromPc = true, pinned = false),
        ClipboardHistoryView(5, true, "Obraz 321×220", image(0xFF5A2D4A.toInt(), 64, 44), fromPc = true, pinned = false),
        ClipboardHistoryView(4, false, "https://www.example.com/artykul/bardzo-dlugi-adres-ktory-sie-nie-miesci", null, fromPc = false, pinned = true),
        ClipboardHistoryView(3, true, "Obraz 221×160", image(0xFFB5531E.toInt(), 64, 46), fromPc = true, pinned = false),
        ClipboardHistoryView(2, false, "C:\\Users\\Ola\\Documents\\Projekty\\Raport kwartalny\\wyniki-2026-Q3.xlsx", null, fromPc = true, pinned = false),
        ClipboardHistoryView(1, false, "Spotkanie o 15:00 w sali B, przynieś laptopa i ładowarkę", null, fromPc = false, pinned = false),
    )

    private fun paired(connected: Boolean) = PairingUi(ready = true, initialized = true, hasPairedDevice = true, computerName = "SZYMON",
        connection = if (connected) ConnectionState.Connected("SZYMON", "key", 2400) else ConnectionState.Disconnected("test"),
        clipboard = ClipboardUi(enabled = true, status = "Schowek odebrany z komputera"), clipboardHistory = history, filesAccepted = true)

    private fun theme(mode: String, dynamic: Boolean) {
        context.getSharedPreferences("appearance", 0).edit().putString("theme", mode).putBoolean("dynamic", dynamic).commit()
    }

    /** One page per screen height, as a phone's screenshots. */
    private fun pages(name: String, scroll: ScrollState, count: Int) {
        for (page in 0 until count) {
            compose.runOnIdle { runBlocking { scroll.scrollTo((page * scroll.viewportSize * 0.9f).toInt().coerceAtMost(scroll.maxValue)) } }
            compose.waitForIdle()
            val shot = compose.onNodeWithTag("shotRoot").captureToImage().asAndroidBitmap()
            save("$name-${page + 1}", shot)
            if (scroll.value >= scroll.maxValue) break
        }
    }

    @Composable
    private fun Screen(scroll: ScrollState, settings: Boolean, content: @Composable ColumnScope.() -> Unit) {
        Box(Modifier.width(412.dp).fillMaxHeight().testTag("shotRoot")) {
            WindApp(forceContent = settings, onSettings = {}) {
                Column(Modifier.fillMaxSize().verticalScroll(scroll).padding(24.dp), verticalArrangement = Arrangement.spacedBy(18.dp), content = content)
            }
        }
    }

    private fun run(name: String, mode: String, dynamic: Boolean, settings: Boolean, content: @Composable ColumnScope.() -> Unit) {
        theme(mode, dynamic)
        lateinit var scroll: ScrollState
        compose.setContent { scroll = rememberScrollState(); Screen(scroll, settings, content) }
        pages(name, scroll, 4)
    }

    @Test fun homeDynamicDarkOffline() { enabled(); run("home-you-offline", "Dark", true, false) {
        HomeScreen(paired(false), {}, {}, onClipboard = {}, onClipboardEnabled = {}, onSendFile = {}, history = ClipboardHistoryActions({}, { _, _ -> }, {}, {}))
    } }

    @Test fun homeBrandDarkConnected() { enabled(); run("home-brand-connected", "Dark", false, false) {
        HomeScreen(paired(true), {}, {}, onClipboard = {}, onClipboardEnabled = {}, onSendFile = {}, history = ClipboardHistoryActions({}, { _, _ -> }, {}, {}))
    } }

    @Test fun homeBrandLightOffline() { enabled(); run("home-brand-light", "Light", false, false) {
        HomeScreen(paired(false), {}, {}, onClipboard = {}, onClipboardEnabled = {}, onSendFile = {}, history = ClipboardHistoryActions({}, { _, _ -> }, {}, {}))
    } }

    @Test fun settingsDynamicDark() { enabled(); run("settings-you", "Dark", true, true) { SettingsSample() } }

    @Test fun settingsBrandLight() { enabled(); run("settings-brand-light", "Light", false, true) { SettingsSample() } }

    @Composable
    private fun SettingsSample() {
        WindBackHeader(str(R.string.settings), {}, "closeSettings")
        WindSection(str(R.string.group_computer)) {
            WindSettingRow("pc", str(R.string.devices_paired), "SZYMON (połączony)", switchTag = "openDevices", onClick = {})
            WindDivider()
            WindSettingRow("plus", str(R.string.pair_another), str(R.string.sum_pair), switchTag = "scanQr", onClick = {})
        }
        WindSection(str(R.string.group_phone_on_pc)) {
            WindSettingRow("bell", str(R.string.notify_title), str(R.string.sum_notify), checked = true, onCheckedChange = {},
                status = str(R.string.needs_access), warning = true,
                actions = {
                    WindButton(str(R.string.access_grant_short), {}, kind = ButtonKind.Secondary)
                    NotifyAppsList(NotifyAppsState(NotifyAppMode.AllExcept, emptySet(), setOf("com.instagram.android", "com.android.systemui", "com.tippingcanoe.pepperpl")),
                        { mapOf("com.instagram.android" to "Instagram", "com.android.systemui" to "Interf. sys.").getOrDefault(it, it) }, {}, { _, _ -> })
                },
                more = { androidx.compose.material3.Text(str(R.string.notify_about)) })
            WindDivider()
            CallControls()
            WindDivider()
            GalleryControls()
        }
        WindSection(str(R.string.group_shortcuts)) {
            TileAddControls(35) {}
            WindDivider()
            LauncherShortcutControls()
            WindDivider()
            WidgetControls()
            WindDivider()
            RelayCard(null) {}
        }
    }
}
