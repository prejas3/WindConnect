package io.github.prejas3.windconnect

import android.content.Intent
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.util.Collections

/**
 * WN-100 review: another app can start OpenOnPcActivity with an explicit intent; nothing may go to the PC until the person taps
 * "Open on PC". Cancel sends nothing; the tap sends exactly the shared link.
 */
@RunWith(AndroidJUnit4::class)
class OpenOnPcTest {
    @get:Rule val compose = createEmptyComposeRule()
    private val sent: MutableList<String> = Collections.synchronizedList(mutableListOf())

    @Before fun fixture() {
        OpenOnPcActivity.testPeer = "Test PC"
        OpenOnPcActivity.testSender = { url -> sent += url; "done" }
    }

    @After fun clear() {
        OpenOnPcActivity.testPeer = null
        OpenOnPcActivity.testSender = null
    }

    private fun explicitShare() = Intent(InstrumentationRegistry.getInstrumentation().targetContext, OpenOnPcActivity::class.java)
        .setAction(Intent.ACTION_SEND).setType("text/plain")
        .putExtra(Intent.EXTRA_TEXT, "zobacz https://example.com/wn100?x=1 teraz")

    @Test fun nothingGoesOutWithoutATap() {
        ActivityScenario.launch<OpenOnPcActivity>(explicitShare()).use {
            compose.onNodeWithText("Otworzyć na Test PC?").assertExists()
            compose.onNodeWithTag("openOnPcLink").assertExists()
            Thread.sleep(1_500)
            assertTrue("sent before any tap: $sent", sent.isEmpty())
            compose.onNodeWithTag("openOnPcCancel").performClick()
            compose.waitForIdle()
            Thread.sleep(500)
            assertTrue("cancel sent something: $sent", sent.isEmpty())
        }
    }

    @Test fun theTapSendsTheLink() {
        ActivityScenario.launch<OpenOnPcActivity>(explicitShare()).use {
            compose.onNodeWithTag("openOnPcConfirm").performClick()
            compose.waitUntil(5_000) { sent.isNotEmpty() }
            assertEquals(listOf("https://example.com/wn100?x=1"), sent.toList())
        }
    }
}
