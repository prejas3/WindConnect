package io.github.prejas3.windconnect

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.withTimeout

/** CCS/1 RULE:Main-thread admission attempts have unique native tokens; late acknowledgements cannot resurrect removed tokens. */
internal class ReceiveAdmission(
    private val enabled: () -> Boolean,
    private val visible: () -> Boolean,
    private val owner: () -> Boolean,
    private val start: () -> Unit,
    private val changed: () -> Unit,
    private val timeoutMillis: Long = 4_000,
) {
    private val attempts = mutableMapOf<ULong, CompletableDeferred<Boolean>>()
    val active: Boolean get() = attempts.isNotEmpty()
    val waiting: Boolean get() = attempts.values.any { !it.isCompleted }

    suspend fun begin(token: ULong, current: () -> Boolean = { true }): Boolean {
        if (!current() || !enabled() || (!owner() && !visible()) || token in attempts) return false
        val ready = CompletableDeferred<Boolean>()
        attempts[token] = ready
        var accepted = false
        try {
            if (!owner()) {
                if (!current() || !enabled() || !visible()) return false
                start()
            }
            ownerChanged()
            accepted = withTimeout(timeoutMillis) { ready.await() } && current() && enabled() && owner()
            return accepted
        } catch (_: TimeoutCancellationException) {
            return false
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (_: RuntimeException) {
            return false
        } finally {
            if (!accepted) end(token)
        }
    }

    fun ownerChanged() {
        val legal = enabled() && owner()
        if (legal || !enabled() || !visible()) attempts.values.forEach { it.complete(legal) }
    }
    fun startupFailed() { attempts.values.forEach { it.complete(false) } }

    fun end(token: ULong) {
        attempts.remove(token)?.let { it.complete(false); changed() }
    }
}
