package io.github.prejas3.windconnect

import android.os.Bundle
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test

class CoreConnectionTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()

    private fun text(tag: String): String = compose.onNodeWithTag(tag).fetchSemanticsNode()
        .config[SemanticsProperties.Text].joinToString("") { it.text }

    private fun identity(): String {
        if (compose.onAllNodesWithTag("localKey").fetchSemanticsNodes().isEmpty()) {
            compose.onNodeWithText("Test połączenia z PC").performClick()
        }
        compose.waitUntil(15_000) { text("localKey").length == 64 }
        return text("localKey")
    }

    @Test fun identityLoadsThroughUniFFIAndSurvivesActivityRecreation() {
        val before = identity()
        compose.activityRule.scenario.recreate()
        assertEquals(before, identity())
        InstrumentationRegistry.getInstrumentation().sendStatus(0, Bundle().apply {
            putString("androidPublicKey", before)
        })
    }

    /** CCS/1 IO:instrumentation pcKey + pcAddress -> real screen -> Rust PC session */
    @Test fun screenConnectsToPc() {
        val args = InstrumentationRegistry.getArguments()
        val key = args.getString("pcKey")
        val address = args.getString("pcAddress")
        assumeTrue("requires pcKey and pcAddress instrumentation arguments", key != null && address != null)
        identity()
        compose.onNodeWithTag("peerAddress").performScrollTo().performTextInput(address!!)
        compose.onNodeWithTag("peerKey").performScrollTo().performTextInput(key!!)
        compose.onNodeWithTag("connect").performScrollTo().performClick()
        compose.waitUntil(20_000) { text("connectionStatus").startsWith("Połączono:") }
        InstrumentationRegistry.getInstrumentation().sendStatus(0, Bundle().apply {
            putString("pcConnected", text("connectionStatus"))
        })
    }
}
