package io.github.prejas3.windconnect

import android.content.ComponentName
import android.content.Intent
import android.content.pm.PackageManager
import android.service.quicksettings.TileService
import android.app.Application
import io.github.prejas3.windconnect.core.ConnectionState
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.flow.first
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Test

class ClipboardTileRegistrationTest {
    @Test fun observesNativeConnectionWithoutActivityAndResetsAfterLastLease() = runBlocking {
        val application = InstrumentationRegistry.getInstrumentation().targetContext.applicationContext as Application
        val owner = ConnectionOwner(application)
        assertEquals(ConnectionState.Stopped, owner.state.value)
        val first = owner.acquire()
        val second = owner.acquire()
        try {
            assertSame(first, second)
            first.disconnect()
            val disconnected = withTimeout(5_000) {
                owner.state.first { it is ConnectionState.Disconnected }
            }
            assertEquals(first.state.value, disconnected)
            owner.release(second)
            assertEquals(disconnected, owner.state.value)
        } finally {
            owner.release(first)
        }
        assertEquals(ConnectionState.Stopped, owner.state.value)
    }

    @Test fun systemUiCanDiscoverTileAndLongPressOpensApp() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val manager = context.packageManager
        @Suppress("DEPRECATION")
        val info = manager.getServiceInfo(ComponentName(context, ClipboardTileService::class.java), 0)
        assertTrue(info.exported)
        assertEquals("android.permission.BIND_QUICK_SETTINGS_TILE", info.permission)
        val tileIntent = Intent(TileService.ACTION_QS_TILE).setPackage(context.packageName)
        @Suppress("DEPRECATION")
        val services = manager.queryIntentServices(tileIntent, 0)
        assertTrue(services.any { it.serviceInfo.name == ClipboardTileService::class.java.name })
        val hold = Intent(TileService.ACTION_QS_TILE_PREFERENCES).setPackage(context.packageName)
        @Suppress("DEPRECATION")
        val target = manager.resolveActivity(hold, PackageManager.MATCH_DEFAULT_ONLY)
        assertEquals(MainActivity::class.java.name, target?.activityInfo?.name)
    }
}
