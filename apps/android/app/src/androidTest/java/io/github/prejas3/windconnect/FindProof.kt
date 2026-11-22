package io.github.prejas3.windconnect

import android.app.NotificationManager
import android.os.Bundle
import android.os.SystemClock
import android.view.KeyEvent
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.AndroidComposeTestRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.rules.ActivityScenarioRule
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue

/**
 * CCS/1 PURPOSE:WN-50 phone side of check-find-android.cjs. Ring 1 is stopped from the PC; then each `findStops` entry stops
 * the next ring: "notification" (the notification's own "Stop" PendingIntent, the receiver a tap reaches), "bar" (the
 * app's bar, with POST_NOTIFICATIONS refused for the run), "volume" (a volume key in the app).
 */
object FindProof {
    fun <A : ComponentActivity> run(compose: AndroidComposeTestRule<ActivityScenarioRule<A>, A>) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = compose.activity.applicationContext
        val connections = (context as WindApplication).connections
        val notifications = context.getSystemService(NotificationManager::class.java)
        fun waitRinging(value: Boolean, label: String) {
            val deadline = SystemClock.elapsedRealtime() + 60_000
            while (connections.ringing.value != value) {
                assertTrue("$label: ringing=$value", SystemClock.elapsedRealtime() < deadline)
                Thread.sleep(20)
            }
        }
        fun ours() = notifications.activeNotifications.firstOrNull { it.id == 50 }
        val report = Bundle()

        // 1. Stopped from the PC.
        waitRinging(true, "ring 1"); waitRinging(false, "ring 1 stopped by the PC")

        // Then the stops the PC script asks for, in order (`-e findStops notification,volume` or `bar,volume`).
        val stops = InstrumentationRegistry.getArguments().getString("findStops").orEmpty().split(',').filter { it.isNotEmpty() }
        for ((index, stop) in stops.withIndex()) {
            val label = "ring ${index + 2} ($stop)"
            waitRinging(true, label)
            when (stop) {
                // The notification: its own channel, high importance, silent, ongoing; its action stops the ring.
                "notification" -> {
                    var posted = ours()
                    val deadline = SystemClock.elapsedRealtime() + 3_000
                    while (posted == null && SystemClock.elapsedRealtime() < deadline) { Thread.sleep(20); posted = ours() }
                    assertNotNull("ring notification posted", posted)
                    val channel = notifications.getNotificationChannel(posted!!.notification.channelId)
                    assertEquals("find_phone", channel.id)
                    assertEquals(NotificationManager.IMPORTANCE_HIGH, channel.importance)
                    assertNull("no sound of its own (AudioTrack plays)", channel.sound)
                    assertTrue("ongoing", posted.isOngoing)
                    assertEquals("Stop", posted.notification.actions.single().title.toString())
                    posted.notification.actions.single().actionIntent.send()
                }
                // Notifications refused: nothing is posted; the app's bar stops it.
                "bar" -> {
                    assertTrue("notifications refused for this run", !notifications.areNotificationsEnabled())
                    Thread.sleep(300)
                    assertNull("no notification while refused", ours())
                    compose.onNodeWithTag("stopRinging").performClick()
                }
                // A volume key while the app is open.
                "volume" -> {
                    compose.waitForIdle()
                    val now = SystemClock.uptimeMillis()
                    for (action in listOf(KeyEvent.ACTION_DOWN, KeyEvent.ACTION_UP)) {
                        instrumentation.uiAutomation.injectInputEvent(KeyEvent(now, now, action, KeyEvent.KEYCODE_VOLUME_DOWN, 0), true)
                    }
                }
                else -> throw AssertionError("unknown stop $stop")
            }
            waitRinging(false, "$label stopped")
            assertNull("notification removed after the final state", ours())
        }

        report.putString("findProof", stops.joinToString(","))
        instrumentation.sendStatus(0, report)
    }
}
