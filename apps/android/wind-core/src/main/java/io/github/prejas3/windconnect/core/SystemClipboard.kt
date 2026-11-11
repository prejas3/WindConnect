package io.github.prejas3.windconnect.core

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.SharedPreferences
import android.os.Handler
import android.os.Looper
import android.os.PersistableBundle
import io.github.prejas3.windconnect.bindings.ClipboardSink
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

data class ClipboardUi(val enabled: Boolean = true, val status: String = "Synchronizacja automatyczna", val busy: Boolean = false, val delivered: Long = 0, val received: Long = 0)

/** CCS/1 SEC:only plain text; never coerce provider URIs/Intents; no clipboard payload in UI/logs/storage. */
internal class SystemClipboard(
    context: Context, private val preferences: SharedPreferences, private val scope: CoroutineScope,
    private val enabledChanged: (Boolean) -> Unit,
    private val connected: () -> Boolean,
    private val publish: suspend (String, Boolean, Boolean) -> String,
) : ClipboardSink {
    private val manager = context.getSystemService(ClipboardManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private val mutable = MutableStateFlow(preferences.getBoolean("clipboard_enabled", true).let { enabled ->
        ClipboardUi(enabled, if (enabled) "Synchronizacja automatyczna" else "Synchronizacja wyłączona")
    })
    val ui = mutable.asStateFlow()
    private var focused = false
    private var listening = false
    @Volatile private var closed = false
    private var sending: Job? = null
    private var dirty = false
    private val freshness = ClipboardFreshness()
    private val listener = ClipboardManager.OnPrimaryClipChangedListener { automatic() }

    fun setEnabled(value: Boolean) {
        enabledChanged(value)
        preferences.edit().putBoolean("clipboard_enabled", value).apply()
        mutable.update { it.copy(enabled = value, status = if (value) "Synchronizacja automatyczna" else "Synchronizacja wyłączona") }
        main.post { updateListener() }
    }
    fun focus(value: Boolean) { main.post { val changed = focused != value; focused = value; updateListener(); if (value && changed) automatic() } }
    fun connectionChanged() { main.post { updateListener() } }
    fun sessionStarted() { freshness.sessionStarted(System.currentTimeMillis()) }
    private fun updateListener() {
        val wanted = !closed && focused && mutable.value.enabled && connected()
        if (wanted && !listening) manager.addPrimaryClipChangedListener(listener)
        if (!wanted && listening) manager.removePrimaryClipChangedListener(listener)
        listening = wanted
    }
    private fun automatic() {
        if (closed || !focused || !mutable.value.enabled || !connected()) return
        dirty = true
        if (sending?.isActive == true) return
        sending = scope.launch(Dispatchers.Main.immediate) {
            do { dirty = false; send(false) } while (dirty && focused && !closed && mutable.value.enabled)
        }
    }
    suspend fun send(manual: Boolean = true): String = withContext(Dispatchers.Main.immediate) {
        if (closed) return@withContext "unavailable"
        if (!connected()) { if (manual) report("unavailable"); return@withContext "unavailable" }
        val clip = try { manager.primaryClip } catch (_: SecurityException) { null }
        if (!manual && clip != null && !freshness.allows(clip.description.timestamp)) return@withContext "unchanged"
        if (clip == null || clip.itemCount != 1 || clip.getItemAt(0).text == null) {
            if (manual) report("empty"); return@withContext "empty"
        }
        if (clip.description.extras?.getBoolean(SENSITIVE, false) == true) {
            if (manual) report("sensitive")
            return@withContext "sensitive"
        }
        val value = clip.getItemAt(0).text
        if (value.length > MAX_TEXT) { if (manual) report("invalid"); return@withContext "invalid" }
        val text = value.toString()
        if (text.contains('\u0000') || text.toByteArray(Charsets.UTF_8).size > MAX_TEXT) { if (manual) report("invalid"); return@withContext "invalid" }
        mutable.update { it.copy(busy = true) }
        try {
            val result = publish(text, false, manual)
            if (result == "applied") mutable.update { it.copy(delivered = it.delivered + 1) }
            if (result != "unchanged" || manual) report(result)
            result
        } finally { mutable.update { it.copy(busy = false) } }
    }
    override fun applyText(text: String): Boolean {
        if (closed || !mutable.value.enabled) return false
        // CCS/1 RULE:Binder clipboard write is synchronous; no UI-thread wait/re-entry into native clipboard state.
        return try {
            val clip = ClipData.newPlainText("WindConnect", text)
            clip.description.extras = PersistableBundle().apply { putBoolean(SENSITIVE, true) }
            manager.setPrimaryClip(clip)
            mutable.update { it.copy(status = "Schowek odebrany z komputera", received = it.received + 1) }
            true
        } catch (_: Exception) { false }
    }
    private fun report(result: String) { mutable.update { it.copy(status = statusText(result)) } }
    fun close() { closed = true; main.post { sending?.cancel(); updateListener() } }
    companion object {
        const val SENSITIVE = "android.content.extra.IS_SENSITIVE"
        const val MAX_TEXT = 256 * 1024
        fun statusText(value: String) = when (value) {
            "applied" -> "Schowek dostarczony do komputera"
            "sensitive" -> "Treść oznaczona jako wrażliwa — nie wysłano"
            "unchanged" -> "Schowek jest już zsynchronizowany"
            "disabled" -> "Synchronizacja automatyczna wyłączona"
            "refused" -> "Komputer odrzucił schowek lub nowszą zmianę już zastosowano"
            "invalid" -> "Obsługiwany jest tekst do 256 KiB, bez znaku NUL"
            "empty" -> "Brak tekstu w schowku lub brak dostępu do niego"
            "busy" -> "Schowek jest zajęty — spróbuj ponownie"
            "failed" -> "Nie udało się zapisać schowka na komputerze"
            else -> "Schowek niedostępny — sprawdź połączenie z komputerem"
        }
    }
}
