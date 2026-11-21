package io.github.prejas3.windconnect

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import androidx.lifecycle.Lifecycle
import org.junit.Rule
import org.junit.Test

/** CCS/1 CHECK:real disconnected Activity, no synthetic permission/connection; layout and lifecycle at 150% font. */
class TouchpadScreenTest {
    @get:Rule val compose=createEmptyComposeRule()
    @Test fun disconnectedScreenStaysSafeAcrossPauseAndRecreationInBothThemes(){
        val context=InstrumentationRegistry.getInstrumentation().targetContext
        val prefs=context.getSharedPreferences("appearance",0)
        val previous=prefs.getString("theme",null)
        try {
            for(theme in listOf("Light","Dark")) {
                prefs.edit().putString("theme",theme).putBoolean("dynamic",false).commit()
                ActivityScenario.launch(TouchpadActivity::class.java).use { activity ->
                    compose.onNodeWithText("Lewy · przytrzymaj").assertIsNotEnabled()
                    compose.onNodeWithText("Prawy").assertIsNotEnabled()
                    compose.onNodeWithTag("remoteKeyboardToggle").assertIsNotEnabled()
                    compose.onNodeWithTag("touchpadSurface").assertIsDisplayed().assertHeightIsAtLeast(100.dp)
                    compose.onNodeWithText("Czułość").assertIsDisplayed()
                    val image=compose.onRoot().captureToImage().asAndroidBitmap()
                    java.io.File(context.cacheDir,"wn51-$theme-150.png").outputStream().use{image.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)}
                    activity.moveToState(Lifecycle.State.CREATED);activity.moveToState(Lifecycle.State.RESUMED);activity.recreate()
                    compose.onNodeWithText("Lewy · przytrzymaj").assertIsNotEnabled()
                    compose.onNodeWithTag("remoteKeyboardToggle").assertIsNotEnabled()
                    compose.onNodeWithTag("closeTouchpad").assertIsDisplayed()
                    // CCS/1 CHECK:WN-53 disconnected: presenter is off and volume keys still reach the system.
                    compose.onNodeWithTag("presenterToggle").assertIsNotEnabled()
                    activity.onActivity { a ->
                        for(code in listOf(android.view.KeyEvent.KEYCODE_VOLUME_UP,android.view.KeyEvent.KEYCODE_VOLUME_DOWN)) {
                            org.junit.Assert.assertFalse(a.onKeyDown(code,android.view.KeyEvent(android.view.KeyEvent.ACTION_DOWN,code)))
                            org.junit.Assert.assertFalse(a.onKeyUp(code,android.view.KeyEvent(android.view.KeyEvent.ACTION_UP,code)))
                        }
                    }
                }
            }
        } finally {prefs.edit().putString("theme",previous).apply()}
    }
}
