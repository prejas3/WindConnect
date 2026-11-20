package io.github.prejas3.windconnect

import android.content.Context
import android.content.res.Configuration
import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.bindings.RemoteAction
import io.github.prejas3.windconnect.ui.WindConnectTheme
import java.util.Locale
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * WN-165: the PC's actions on the phone are named in the phone's language by their id, whatever language the PC sent
 * (an older PC sends Polish labels); a custom action and an id this version doesn't know keep the PC's label.
 */
@RunWith(AndroidJUnit4::class)
class PcActionNamesTest {
    @get:Rule val compose = createComposeRule()
    private val app: Context = InstrumentationRegistry.getInstrumentation().targetContext
    private val kept = Strings.context

    // As an older PC (0.81.0) lists them: Polish labels, whatever the phone's language.
    private val fromPc = listOf(
        RemoteAction("sleep", "Uśpij"),
        RemoteAction("x-backup", "Kopia zapasowa"),
        RemoteAction("hibernate", "Hibernate"),
    )

    @After fun restore() { Strings.context = kept }

    private fun language(tag: String): Context =
        app.createConfigurationContext(Configuration(app.resources.configuration).apply { setLocale(Locale.forLanguageTag(tag)) })

    @Test fun namesFollowThePhonesLanguage() {
        val ran = mutableListOf<Pair<String, String>>()
        val turns = mutableIntStateOf(0)
        Strings.context = language("en-US")
        compose.setContent {
            val turn by turns
            WindConnectTheme { Column { key(turn) { PcActionButtons(fromPc) { id, label -> ran += id to label } } } }
        }
        compose.onNodeWithTag("pcAction:sleep").assertTextEquals("Sleep PC")
        compose.onNodeWithTag("pcAction:x-backup").assertTextEquals("Kopia zapasowa")
        compose.onNodeWithTag("pcAction:hibernate").assertTextEquals("Hibernate")
        compose.onNodeWithTag("pcAction:sleep").performClick()
        compose.runOnIdle { assertEquals(listOf("sleep" to "Sleep PC"), ran) }

        Strings.context = language("pl-PL")
        compose.runOnIdle { turns.intValue = 1 }
        compose.onNodeWithTag("pcAction:sleep").assertTextEquals("Uśpij PC")
        compose.onNodeWithTag("pcAction:x-backup").assertTextEquals("Kopia zapasowa")
        compose.onNodeWithTag("pcAction:hibernate").assertTextEquals("Hibernate")
    }
}
