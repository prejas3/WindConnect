package io.github.prejas3.windconnect

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.os.Bundle
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.core.NotifyListenerService
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeNotNull
import org.junit.Test

/** CCS/1 FLOW:real call notification->consent off->cancel while unbound->consent on->new real session. */
class CallNotificationLifecycleTest {
    @Test fun aMissedRemovalCannotRingInTheNextSession() = runBlocking {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val args = InstrumentationRegistry.getArguments()
        val key = args.getString("pcKey")
        val address = args.getString("pcAddress")
        assumeNotNull(key, address)
        val app = context.applicationContext as WindApplication
        val core = app.connections.acquire()
        val manager = context.getSystemService(NotificationManager::class.java)
        val channel = "call-lifecycle-proof"
        suspend fun waitCount(expected: Int) = withTimeout(10_000) {
            while (CallNotificationProbe.count() != expected) delay(50)
        }
        try {
            assertTrue(NotifyListenerService.granted(context))
            core.setNotifyAllowed(true)
            delay(1_500)
            manager.createNotificationChannel(NotificationChannel(channel, "Call lifecycle proof", NotificationManager.IMPORTANCE_LOW))
            manager.notify(123, Notification.Builder(context, channel)
                .setSmallIcon(android.R.drawable.sym_call_incoming).setContentTitle("Synthetic call")
                .setCategory(Notification.CATEGORY_CALL).build())
            waitCount(1)
            core.setNotifyAllowed(false)
            assertEquals("consent off clears immediately", 0, CallNotificationProbe.count())
            delay(1_000)
            manager.cancel(123)
            delay(500)
            core.setNotifyAllowed(true)
            delay(1_500)
            assertEquals("rebind has no stale call", 0, CallNotificationProbe.count())
            val bytes = ByteArray(key!!.length / 2) { key.substring(it * 2, it * 2 + 2).toInt(16).toByte() }
            core.connect(address!!, bytes)
            delay(3_000)
            instrumentation.sendStatus(0, Bundle().apply { putString("callLifecycle", "new session after missed removal") })
        } finally {
            manager.cancel(123)
            manager.deleteNotificationChannel(channel)
            core.setNotifyAllowed(false)
            core.disconnect()
            app.connections.release(core)
        }
    }
}
