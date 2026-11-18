package io.github.prejas3.windconnect

import android.app.Instrumentation
import android.content.Intent
import android.content.IntentFilter
import android.net.Uri
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

/**
 * WN-156: the "file received" notice's tap (OpenReceivedActivity) opens the file from the receive journal for a known
 * receipt, and the received-files list for an unknown one. Monitors block the targets, so nothing else starts.
 */
@RunWith(AndroidJUnit4::class)
class OpenReceivedTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val journal = Uri.parse("content://io.github.prejas3.windconnect.test.none/42")
    private val monitors = mutableListOf<Instrumentation.ActivityMonitor>()

    @After fun clean() { OpenReceivedActivity.testUri = null; monitors.forEach(instrumentation::removeMonitor) }

    private fun open(key: String) = context.startActivity(Intent(context, OpenReceivedActivity::class.java)
        .putExtra(OpenReceivedActivity.KEY, key).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))

    /** A blocked start counts a hit but creates no activity, so wait on the hits. */
    private fun waitFor(done: () -> Boolean) {
        val until = android.os.SystemClock.uptimeMillis() + 5_000
        while (!done() && android.os.SystemClock.uptimeMillis() < until) Thread.sleep(50)
    }

    @Test fun aKnownReceiptOpensTheFile() {
        OpenReceivedActivity.testUri = { key -> if (key == "known") journal else null }
        val view = instrumentation.addMonitor(IntentFilter(Intent.ACTION_VIEW).apply { addDataScheme("content"); addDataType("*/*") }, null, true).also(monitors::add)
        val list = instrumentation.addMonitor(ReceiveFilesActivity::class.java.name, null, true).also(monitors::add)
        open("known")
        waitFor { view.hits > 0 || list.hits > 0 }
        assertEquals(1, view.hits)
        assertEquals(0, list.hits)
    }

    @Test fun anUnknownReceiptOpensTheList() {
        OpenReceivedActivity.testUri = { null }
        val view = instrumentation.addMonitor(IntentFilter(Intent.ACTION_VIEW).apply { addDataScheme("content"); addDataType("*/*") }, null, true).also(monitors::add)
        val list = instrumentation.addMonitor(ReceiveFilesActivity::class.java.name, null, true).also(monitors::add)
        open("gone")
        waitFor { view.hits > 0 || list.hits > 0 }
        assertEquals(1, list.hits)
        assertEquals(0, view.hits)
    }
}
