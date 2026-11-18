package io.github.prejas3.windconnect

/**
 * WN-130/WN-156: which receipts the "file received" notice is about (one: its tap opens the file; several: the list).
 * Nothing while a batch is running or when the batch ended with an error ([lastResult]). [batchBase] is the receipt keys
 * known when the batch began; without it (the batch started before this process saw it) only the newest counts as new.
 */
internal fun <T> freshReceipts(receipts: List<T>, key: (T) -> String, batchBase: Set<String>?, busy: Boolean, lastResult: String): List<T> {
    if (busy || lastResult.isNotEmpty()) return emptyList()
    val known = batchBase ?: receipts.drop(1).map(key).toSet()
    return receipts.filter { key(it) !in known }
}
