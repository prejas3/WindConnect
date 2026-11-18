package io.github.prejas3.windconnect.core

import android.app.Notification
import android.content.pm.ApplicationInfo
import android.os.Bundle
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * WN-159: an app hidden by package visibility is named from the ApplicationInfo its notification carries
 * ("android.appInfo"), the name is kept for later (media card, settings list), and the package name is only the last resort.
 */
@RunWith(AndroidJUnit4::class)
class AppLabelsTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val pepper = "com.tippingcanoe.pepperpl.wn159test"

    @Before fun clean() = AppLabels.forgetForTests(context)
    @After fun cleanAfter() = AppLabels.forgetForTests(context)

    private fun info(pkg: String, label: String) = ApplicationInfo().apply { packageName = pkg; nonLocalizedLabel = label }

    private fun notification(info: ApplicationInfo?): Notification = Notification().apply {
        extras = Bundle().apply { if (info != null) putParcelable("android.appInfo", info) }
    }

    @Test fun aHiddenAppIsNamedFromItsNotification() {
        assertEquals("no name known yet: the package name", pepper, AppLabels.label(context, pepper))
        val carried = AppLabels.info(notification(info(pepper, "Pepper")), pepper)
        AppLabels.learn(context, pepper, carried)
        assertEquals("Pepper", AppLabels.label(context, pepper))
        assertEquals(pepper, AppLabels.infoFor(context, pepper)?.packageName)
    }

    @Test fun theNameIsKeptAfterTheNotification() {
        AppLabels.learn(context, pepper, info(pepper, "Pepper"))
        // A new process: the in-memory infos are gone, the kept name stays.
        val kept = context.getSharedPreferences("app-labels", android.content.Context.MODE_PRIVATE).getString(pepper, null)
        assertEquals("Pepper", kept)
    }

    @Test fun anInfoForAnotherPackageDoesNotCount() {
        assertNull(AppLabels.info(notification(info("com.bank.real", "Bank")), pepper))
        AppLabels.learn(context, pepper, info("com.bank.real", "Bank"))
        assertEquals(pepper, AppLabels.label(context, pepper))
    }

    @Test fun noInfoMeansThePackageName() {
        assertNull(AppLabels.info(notification(null), pepper))
        assertEquals(pepper, AppLabels.label(context, pepper))
    }

    @Test fun aVisibleAppUsesItsInstalledName() {
        // "android" is always visible; its installed name wins over anything learned.
        AppLabels.learn(context, "android", info("android", "Udawany"))
        val name = AppLabels.label(context, "android")
        assertNotEquals("android", name)
        assertNotEquals("Udawany", name)
    }
}
