package io.github.prejas3.windconnect

import org.junit.Assert.assertEquals
import org.junit.Test

/** WN-156: the "file received" notice opens the file for one new receipt, the list for several, nothing after a failure. */
class ReceiveNoticeTest {
    private fun fresh(receipts: List<String>, base: Set<String>?, busy: Boolean = false, lastResult: String = "") =
        freshReceipts(receipts, { it }, base, busy, lastResult)

    @Test fun oneNewFile() = assertEquals(listOf("c"), fresh(listOf("c", "b", "a"), setOf("a", "b")))
    @Test fun severalNewFiles() = assertEquals(listOf("d", "c"), fresh(listOf("d", "c", "b", "a"), setOf("a", "b")))
    @Test fun failedBatchIsNotAboutAFile() = assertEquals(emptyList<String>(), fresh(listOf("c", "b"), setOf("b"), lastResult = "Couldn't receive the file"))
    @Test fun runningBatchIsNotAboutAFile() = assertEquals(emptyList<String>(), fresh(listOf("c", "b"), setOf("b"), busy = true))
    @Test fun withoutBatchBaseOnlyTheNewest() = assertEquals(listOf("c"), fresh(listOf("c", "b", "a"), null))
    @Test fun withoutBatchBaseAndNoReceipts() = assertEquals(emptyList<String>(), fresh(emptyList(), null))
}
