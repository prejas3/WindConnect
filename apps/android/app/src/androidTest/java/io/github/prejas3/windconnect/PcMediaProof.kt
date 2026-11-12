package io.github.prejas3.windconnect

import android.app.Notification
import android.app.NotificationManager
import android.content.ComponentName
import android.media.session.MediaController
import android.media.session.MediaSession
import android.media.session.MediaSessionManager
import android.os.Bundle
import android.os.SystemClock
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.AndroidComposeTestRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.rules.ActivityScenarioRule
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.core.MediaListenerService
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue

/**
 * CCS/1 PURPOSE:WN-47 phone side of check-pc-media-android.cjs. The PC follows only its test SMTC player. Here: the PC's
 * player shows in the app card and in our MediaSession + MediaStyle notification; 10 pause/play, alternating Android's media
 * controls (the notification's session token, as the system UI uses it) and the app's button, each timed until the PC's
 * changed state comes back (round trip); NEXT disallowed -> 3 commands, no change; the switch off -> session released and
 * notification gone; the PC player closing -> the same. Stages go to the PC script as status lines `pcMediaStage=...`.
 */
object PcMediaProof {
    fun <A : ComponentActivity> run(compose: AndroidComposeTestRule<ActivityScenarioRule<A>, A>) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = compose.activity.applicationContext
        val connections = (context as WindApplication).connections
        val notifications = context.getSystemService(NotificationManager::class.java)
        val sessions = context.getSystemService(MediaSessionManager::class.java)
        val listener = ComponentName(context, MediaListenerService::class.java)
        fun stage(name: String) = instrumentation.sendStatus(0, Bundle().apply { putString("pcMediaStage", name) })
        fun waitFor(label: String, limitMs: Long = 30_000, done: () -> Boolean) {
            val deadline = SystemClock.elapsedRealtime() + limitMs
            while (!done()) { assertTrue(label, SystemClock.elapsedRealtime() < deadline); Thread.sleep(5) }
        }
        fun ours() = notifications.activeNotifications.firstOrNull { it.id == 47 }
        fun ownSessions() = sessions.getActiveSessions(listener).count { it.packageName == context.packageName }
        val report = Bundle()

        waitFor("the PC's player arrives", 60_000) { connections.pcMedia.value?.title == "PC proof song 1" }
        val shown = connections.pcMedia.value!!
        assertEquals("WindConnect PC test", shown.artist)
        assertEquals(200_000L, shown.durationMs)
        assertNotNull("art decoded", shown.art)
        assertTrue("art ≤ 256 px from the PC", shown.art!!.width <= 256)
        waitFor("media notification") { ours() != null }
        val posted = ours()!!
        assertEquals("pc_media", posted.notification.channelId)
        assertEquals(NotificationManager.IMPORTANCE_LOW, notifications.getNotificationChannel("pc_media").importance)
        @Suppress("DEPRECATION")
        val token = posted.notification.extras.getParcelable<MediaSession.Token>(Notification.EXTRA_MEDIA_SESSION)
        assertNotNull("MediaStyle carries our session", token)
        val system = MediaController(context, token!!)
        assertEquals("PC proof song 1", system.metadata?.description?.title?.toString())
        assertEquals("one session of ours", 1, ownSessions())
        // The app's card (on Home).
        if (compose.onAllNodesWithTag("pairingDone").fetchSemanticsNodes().isNotEmpty()) compose.onNodeWithTag("pairingDone").performScrollTo().performClick()
        compose.waitUntil(10_000) { compose.onAllNodesWithTag("pcMediaTitle").fetchSemanticsNodes().isNotEmpty() }
        stage("shown")

        val times = mutableListOf<Long>()
        repeat(10) { i ->
            val before = connections.pcMedia.value!!.playing
            val start = SystemClock.elapsedRealtime()
            if (i % 2 == 0) {
                if (before) system.transportControls.pause() else system.transportControls.play()
            } else {
                compose.onNodeWithTag("pcMediaToggle").performScrollTo().performClick()
            }
            waitFor("toggle $i") { connections.pcMedia.value?.playing == !before }
            times += SystemClock.elapsedRealtime() - start
        }
        report.putString("roundTripMs", times.joinToString(","))
        stage("toggled")

        // The PC script turns NEXT off on its player: our commands for it must change nothing.
        waitFor("NEXT no longer allowed", 30_000) { (connections.pcMedia.value?.actions ?: 4L) and 4L == 0L }
        val title = connections.pcMedia.value!!.title
        repeat(3) { connections.pcMediaCommand("next"); Thread.sleep(300) }
        Thread.sleep(1_500)
        assertEquals("a command outside `actions` gets no reaction", title, connections.pcMedia.value?.title)
        stage("disallowed")

        // The switch off: released and cancelled; on again: back.
        connections.setPcMediaAllowed(false)
        waitFor("switch off clears") { connections.pcMedia.value == null && ours() == null }
        waitFor("no session of ours after the switch") { ownSessions() == 0 }
        connections.setPcMediaAllowed(true)
        waitFor("switch on brings it back") { connections.pcMedia.value != null && ours() != null }
        stage("switched")

        // The PC player closes: Gone -> released, cancelled.
        waitFor("PC player gone", 60_000) { connections.pcMedia.value == null }
        waitFor("cleaned up after Gone") { ours() == null && ownSessions() == 0 }
        report.putString("pcMediaProof", "notification,app,disallowedIgnored,switchCleanup,goneCleanup")
        instrumentation.sendStatus(0, report)
    }
}
