package io.github.prejas3.windconnect

import android.Manifest
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.core.declaresPermission
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

/**
 * WN-124 (decision D2): "SMS on the computer" and "Calls on the computer" are in Settings exactly
 * when the APK declares their permissions: the full (GitHub/F-Droid) build has both, the Play build
 * (`-PwindStore=play`) neither. Run on each build; `-e windStore github|play` also checks which one.
 */
class StoreVariantTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()

    @Test fun smsAndCallSettingsFollowTheDeclaredPermissions() {
        val context = compose.activity
        val sms = context.declaresPermission(Manifest.permission.READ_SMS)
        val calls = context.declaresPermission(Manifest.permission.READ_CALL_LOG)
        val contacts = context.declaresPermission(Manifest.permission.READ_CONTACTS)
        assertEquals("one build has all of SMS, Call Log and contacts, the other none", sms, calls)
        assertEquals(sms, contacts)
        InstrumentationRegistry.getArguments().getString("windStore")?.let { store ->
            assertEquals("windStore=$store", store == "github", sms)
        }
        compose.onNodeWithTag("openSettings").performClick()
        compose.onNodeWithTag("backgroundStatus").assertExists()
        fun shown(tag: String) = compose.onAllNodesWithTag(tag, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        assertEquals("SMS on the computer", sms, shown("smsAllowed"))
        assertEquals("Calls on the computer", calls, shown("callsAllowed"))
        InstrumentationRegistry.getInstrumentation().sendStatus(0, android.os.Bundle().apply {
            putString("storeVariant", if (sms) "full" else "play")
        })
    }
}
