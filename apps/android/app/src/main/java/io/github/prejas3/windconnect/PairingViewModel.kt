package io.github.prejas3.windconnect

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import io.github.prejas3.windconnect.bindings.QrOffer
import io.github.prejas3.windconnect.bindings.inspectQrLink
import io.github.prejas3.windconnect.core.WindCore
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

data class PairingUi(
    val screen: String = "home",
    val ready: Boolean = false,
    val name: String = "",
    val fingerprint: String = "",
    val message: String = "",
    val skippedRecords: ULong = 0uL,
)

class PairingViewModel(application: Application) : AndroidViewModel(application) {
    private val mutableUi = MutableStateFlow(PairingUi())
    val ui = mutableUi.asStateFlow()
    private var offer: QrOffer? = null
    private var core: WindCore? = null

    init {
        viewModelScope.launch {
            try {
                val opened = WindCore.open(application, android.os.Build.MODEL.take(64))
                core = opened
                mutableUi.update { it.copy(ready = true, skippedRecords = opened.skippedPairingRecords) }
            } catch (error: Exception) {
                if (error is CancellationException) throw error
                mutableUi.update { it.copy(message = "Nie udało się odczytać tożsamości urządzenia. Uruchom aplikację ponownie.") }
            }
        }
    }

    fun scan() { if (ui.value.screen != "pairing") { clearOffer(); mutableUi.update { it.copy(screen = "scanner", message = "") } } }
    fun cancel() { if (ui.value.screen != "pairing") { clearOffer(); mutableUi.update { it.copy(screen = "home", name = "", fingerprint = "", message = "") } } }
    fun cameraError() { mutableUi.update { it.copy(screen = "error", message = "Nie udało się otworzyć aparatu. Zamknij inne aplikacje korzystające z aparatu i spróbuj ponownie.") } }

    fun offerLink(link: String?) {
        // CCS/1 SEC:incoming intents cannot replace a pairing in progress; no networking before confirm().
        if (ui.value.screen == "pairing") return
        clearOffer()
        try {
            require(link != null && link.length <= 1024)
            val candidate = inspectQrLink(link)
            offer = candidate
            val preview = candidate.preview()
            mutableUi.update { it.copy(screen = "confirm", name = preview.name, fingerprint = preview.fingerprint, message = "") }
        } catch (_: Exception) {
            clearOffer()
            mutableUi.update { it.copy(screen = "error", name = "", fingerprint = "", message = "To nie jest poprawny kod parowania WindConnect. Wyświetl nowy kod na komputerze.") }
        }
    }

    fun confirm() {
        val active = offer ?: return
        val owner = core ?: return
        if (ui.value.screen != "confirm") return
        offer = null
        mutableUi.update { it.copy(screen = "pairing", message = "") }
        viewModelScope.launch {
            try {
                val peer = owner.pairQr(active)
                mutableUi.update { it.copy(screen = "paired", name = peer.name, fingerprint = peer.fingerprint) }
            } catch (error: Exception) {
                if (error is CancellationException) throw error
                mutableUi.update { it.copy(screen = "error", message = "Nie udało się sparować. Sprawdź wspólną sieć Wi-Fi i zeskanuj nowy kod na komputerze.") }
            } finally { active.destroy() }
        }
    }

    private fun clearOffer() { offer?.destroy(); offer = null }
    override fun onCleared() {
        clearOffer()
        val owner = core
        if (owner != null) CoroutineScope(Dispatchers.IO).launch { owner.shutdown() }
    }
}
