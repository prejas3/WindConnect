package io.github.prejas3.windconnect

import android.content.ContentResolver
import android.graphics.Bitmap
import android.graphics.Color
import android.graphics.Point
import android.os.Bundle
import android.util.Size
import androidx.test.platform.app.InstrumentationRegistry
import io.github.prejas3.windconnect.bindings.ClipboardImageInterface
import io.github.prejas3.windconnect.bindings.CoreException
import io.github.prejas3.windconnect.core.ClipImageProvider
import org.junit.After
import org.junit.Assert.*
import org.junit.Test
import java.io.ByteArrayOutputStream
import java.io.FileNotFoundException
import java.io.IOException
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/**
 * WN-36: received clipboard images through the provider. A preview (EXTRA_SIZE) gets the thumbnail at once; a paste gets
 * the full PNG as it arrives; a replaced image fails the paste (never a partial image); an unknown or forgotten image
 * (e.g. after a restart) is FileNotFoundException, not a crash.
 */
class ClipImageProviderTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val resolver = context.contentResolver

    /** Stands in for the core's ClipboardImage: bytes arrive with [release]; [fail] ends it as a newer item would. */
    private class FakeImage(private val id: String, private val full: ByteArray, private val thumb: ByteArray) : ClipboardImageInterface {
        private val lock = Object()
        private var available = 0
        private var failed = false
        fun release(count: Int) = synchronized(lock) { available = minOf(full.size, available + count); lock.notifyAll() }
        fun fail() = synchronized(lock) { failed = true; lock.notifyAll() }
        override fun id() = id
        override fun size() = full.size.toULong()
        override fun width() = 64uL
        override fun height() = 48uL
        override fun thumbnail() = thumb
        override fun isComplete() = synchronized(lock) { available == full.size }
        override fun read(offset: ULong, max: UInt): ByteArray = synchronized(lock) {
            val deadline = System.currentTimeMillis() + 5_000
            while (true) {
                if (failed) throw CoreException.Transport("image replaced or broken off")
                if (offset.toInt() < available) return full.copyOfRange(offset.toInt(), minOf(available, offset.toInt() + max.toInt()))
                if (available == full.size) return ByteArray(0)
                val left = deadline - System.currentTimeMillis()
                if (left <= 0) throw CoreException.Transport("image transfer stalled")
                lock.wait(left)
            }
            @Suppress("UNREACHABLE_CODE") ByteArray(0)
        }
    }

    private fun png(width: Int, height: Int, color: Int): ByteArray {
        val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888).apply { eraseColor(color) }
        return ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
    }

    @After fun clear() = ClipImageProvider.publish(null)

    @Test fun previewGetsTheThumbnailAndAPasteTheFullPngAsItArrives() {
        val full = png(640, 480, Color.BLUE)
        val thumb = png(64, 48, Color.RED)
        val image = FakeImage("a".repeat(32), full, thumb)
        ClipImageProvider.publish(image)
        val uri = ClipImageProvider.uri(context, image)
        assertEquals("image/png", resolver.getType(uri))

        // A preview asks with a size: the thumbnail, at once (nothing of the full image is in yet).
        val options = Bundle().apply { putParcelable(ContentResolver.EXTRA_SIZE, Point(96, 96)) }
        val preview = resolver.openTypedAssetFileDescriptor(uri, "image/*", options)!!.createInputStream().use { it.readBytes() }
        assertArrayEquals(thumb, preview)
        val bitmap = resolver.loadThumbnail(uri, Size(96, 96), null)
        assertEquals(Color.RED, bitmap.getPixel(0, 0))

        // A paste opened before the bytes are in gets them all once they arrive.
        val pool = Executors.newSingleThreadExecutor()
        val pasted = pool.submit<ByteArray> { resolver.openInputStream(uri)!!.use { it.readBytes() } }
        Thread.sleep(150)
        assertFalse("still waiting for bytes", pasted.isDone)
        image.release(full.size / 2)
        Thread.sleep(50)
        image.release(full.size)
        assertArrayEquals(full, pasted.get(5, TimeUnit.SECONDS))
        pool.shutdown()
    }

    @Test fun aReplacedImageFailsThePasteNeverAPartialImage() {
        val full = png(640, 480, Color.GREEN)
        val image = FakeImage("b".repeat(32), full, png(8, 8, Color.GREEN))
        ClipImageProvider.publish(image)
        val uri = ClipImageProvider.uri(context, image)
        image.release(full.size / 3)
        val pool = Executors.newSingleThreadExecutor()
        val pasted = pool.submit<Result<ByteArray>> { runCatching { resolver.openInputStream(uri)!!.use { it.readBytes() } } }
        Thread.sleep(150)
        // A newer item: the core fails the old image and the provider moves on.
        ClipImageProvider.publish(FakeImage("c".repeat(32), byteArrayOf(1), byteArrayOf(1)))
        image.fail()
        val result = pasted.get(5, TimeUnit.SECONDS)
        assertTrue("the paste fails: $result", result.exceptionOrNull() is IOException)
        pool.shutdown()
    }

    @Test fun anUnknownImageIsNotFoundNotACrash() {
        // After a restart the clip may still name an image this process never had.
        ClipImageProvider.publish(null)
        val uri = ClipImageProvider.uri(context, FakeImage("d".repeat(32), byteArrayOf(1), byteArrayOf(1)))
        assertNull(resolver.getType(uri))
        try {
            resolver.openInputStream(uri)
            fail("expected FileNotFoundException")
        } catch (_: FileNotFoundException) {}
        assertNull(resolver.query(uri, null, null, null, null))
    }
}
