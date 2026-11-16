package io.github.prejas3.windconnect

import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

class PresenterUiTest {
    @get:Rule val compose = createComposeRule()

    @Test fun slideButtonsAndHeldLaserReachTheirCallbacks() {
        val slides = mutableListOf<Long>()
        val laser = mutableListOf<Boolean>()
        val timer = PresentationTimer()
        compose.setContent { MaterialTheme { PresenterPanel(true, true, true, timer, 0, { slides += it }) { laser += it } } }
        compose.onNodeWithTag("presenterNext").performClick()
        compose.onNodeWithTag("presenterPrevious").performClick()
        compose.onNodeWithTag("presenterLaser").assertIsEnabled().performTouchInput { down(center) }
        compose.onNodeWithText("Wskaźnik włączony · celuj telefonem").assertExists()
        compose.onNodeWithTag("presenterLaser").performTouchInput { up() }
        compose.runOnIdle {
            assertEquals(listOf(PresenterKeys.NEXT, PresenterKeys.PREVIOUS), slides)
            assertEquals(listOf(true, false), laser)
        }
        compose.onNodeWithTag("presenterTimer").assertTextEquals("00:00")
        compose.onNodeWithTag("presenterTimerToggle").performClick()
        compose.onNodeWithText("Pauza").assertExists()
    }

    @Test fun oldPcKeepsSlidesButDisablesLaser() {
        val laser = mutableListOf<Boolean>()
        compose.setContent { MaterialTheme { PresenterPanel(true, false, true, PresentationTimer(), 0, {}) { laser += it } } }
        compose.onNodeWithTag("presenterNext").assertIsEnabled()
        compose.onNodeWithTag("presenterLaser").assertIsNotEnabled()
        compose.onNodeWithText("Wskaźnik laserowy wymaga nowszej wersji WindConnect na komputerze.").assertExists()
        compose.runOnIdle { assertTrue(laser.isEmpty()) }
    }
}
