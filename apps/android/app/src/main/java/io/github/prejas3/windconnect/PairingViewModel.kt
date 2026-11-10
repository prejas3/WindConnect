package io.github.prejas3.windconnect

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import io.github.prejas3.windconnect.bindings.DiscoveredDevice
import io.github.prejas3.windconnect.bindings.QrOffer
import io.github.prejas3.windconnect.bindings.SasPairing
import io.github.prejas3.windconnect.bindings.inspectQrLink
import io.github.prejas3.windconnect.core.WindCore
import io.github.prejas3.windconnect.core.ConnectionState
import io.github.prejas3.windconnect.core.ClipboardUi
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
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
    val hasPairedDevice: Boolean = false,
    val initialized: Boolean = false,
    val computerName: String? = null,
    val connection: ConnectionState? = null,
    /** Computers with a code-pairing window open (mDNS TXT `pair`), WN-27. */
    val nearby: List<DiscoveredDevice> = emptyList(),
    /** The code to compare with the computer's screen. */
    val code: String = "",

    val clipboard: ClipboardUi = ClipboardUi(),
)

/** Short, stable label for an unnamed computer in the nearby list (fingerprint prefix). */
fun shortFingerprint(hex: String): String = hex.take(8).uppercase().chunked(4).joinToString(" ")

class PairingViewModel(application: Application) : AndroidViewModel(application) {
    private val mutableUi = MutableStateFlow(PairingUi())
    val ui = mutableUi.asStateFlow()
    private var offer: QrOffer? = null
    private var core: WindCore? = null
    private var sas: SasPairing? = null
    private var nearbyWatch: Job? = null

    init {
        viewModelScope.launch {
            try {
                val opened = (application as WindApplication).connections.acquire()
                core = opened
                viewModelScope.launch { opened.clipboardUi.collect { value -> mutableUi.update { it.copy(clipboard = value) } } }
                val paired = opened.pairedDevices()
                mutableUi.update { it.copy(ready = true, initialized = true, skippedRecords = opened.skippedPairingRecords, hasPairedDevice = paired.isNotEmpty(), computerName = paired.lastOrNull()?.name) }
                viewModelScope.launch {
                    opened.state.collect { state ->
                        val message = when (state) {
                            is ConnectionState.Connected -> "Połączono z komputerem"
                            is ConnectionState.Disconnected -> "Komputer rozłączony · sparowanie zachowane"
                            is ConnectionState.Failed -> "Komputer chwilowo nieosiągalny · sparowanie zachowane"
                            else -> null
                        }
                        mutableUi.update { current -> current.copy(
                            connection = state,
                            computerName = (state as? ConnectionState.Connected)?.peerName ?: current.computerName,
                            message = if (message != null && current.screen in listOf("home", "paired")) message else current.message,
                        ) }
                    }
                }
            } catch (error: Exception) {
                if (error is CancellationException) throw error
                mutableUi.update { it.copy(initialized = true, message = "Nie udało się odczytać tożsamości urządzenia. Uruchom aplikację ponownie.") }
            }
        }
    }

    fun scan() { if (ui.value.screen != "pairing") { clearOffer(); stopNearby(); mutableUi.update { it.copy(screen = "scanner", message = "") } } }
    fun settings() { if (ui.value.screen == "home") mutableUi.update { it.copy(screen = "settings") } }
    fun cancel() { if (ui.value.screen != "pairing" && ui.value.screen != "sas-confirming") { clearOffer(); clearSas(); stopNearby(); mutableUi.update { it.copy(screen = "home", name = "", fingerprint = "", code = "", message = if (core?.state?.value is ConnectionState.Connected) "Połączono z komputerem" else "") } } }

    // CCS/1 FLOW:"Paruj bez aparatu" -> nearby (TXT pair, untrusted hints) -> start_sas -> code on both screens -> both accept -> paired (WN-27)
    fun nearby() {
        val owner = core ?: return
        if (ui.value.screen == "pairing") return
        clearOffer(); clearSas()
        mutableUi.update { it.copy(screen = "nearby", message = "", code = "") }
        if (nearbyWatch?.isActive == true) return
        owner.discoveryScreen(true)
        owner.refreshDiscovery()
        nearbyWatch = viewModelScope.launch {
            try {
                owner.network.collect { snapshot ->
                    mutableUi.update { it.copy(nearby = snapshot.peers.filter { peer -> peer.pairingPort != null && peer.addresses.isNotEmpty() }) }
                }
            } finally { owner.discoveryScreen(false) }
        }
    }

    fun pairWith(device: DiscoveredDevice) {
        val owner = core ?: return
        if (ui.value.screen != "nearby") return
        stopNearby()
        mutableUi.update { it.copy(screen = "sas-connecting", name = "", fingerprint = device.fingerprint, code = "", message = "") }
        viewModelScope.launch {
            try {
                val attempt = owner.startSas(device)
                sas = attempt
                val shown = attempt.code()
                mutableUi.update { it.copy(screen = "sas-code", name = shown.peerName, code = shown.code) }
            } catch (error: Exception) {
                if (error is CancellationException) throw error
                clearSas()
                if (ui.value.screen == "sas-connecting") mutableUi.update { it.copy(screen = "error", code = "", message = "Nie udało się połączyć z komputerem. Na komputerze wybierz „Paruj bez aparatu” i spróbuj ponownie.") }
            }
        }
    }

    /** The person compared both screens: [same] only if the codes match. */
    fun decide(same: Boolean) {
        val attempt = sas ?: return
        val owner = core ?: return
        if (ui.value.screen != "sas-code") return
        mutableUi.update { it.copy(screen = if (same) "sas-confirming" else "sas-rejected", message = "") }
        viewModelScope.launch {
            try {
                val peer = attempt.decide(same)
                mutableUi.update { it.copy(screen = "paired", name = peer.name, computerName = peer.name, fingerprint = peer.fingerprint, code = "", hasPairedDevice = true, message = "Sparowano · Łączenie z komputerem…") }
                try {
                    owner.connectPaired(peer)
                    mutableUi.update { it.copy(message = "Połączono z komputerem") }
                } catch (error: Exception) {
                    if (error is CancellationException && error !is kotlinx.coroutines.TimeoutCancellationException) throw error
                    mutableUi.update { it.copy(message = "Sparowano. Komputer jest chwilowo nieosiągalny — sprawdź wspólną sieć Wi-Fi.") }
                }
                ConnectionService.start(getApplication(), enable = true)
            } catch (error: Exception) {
                if (error is CancellationException) throw error
                mutableUi.update { it.copy(screen = "error", code = "", message = if (same) "Parowanie nie zostało potwierdzone na komputerze. Spróbuj ponownie." else "Kody się różniły, więc urządzenia nie zostały sparowane. Spróbuj ponownie.") }
            } finally { clearSas() }
        }
    }

    private fun stopNearby() { nearbyWatch?.cancel(); nearbyWatch = null }
    // CCS/1 RULE:cancel only; a code()/decide() call may still hold the object, so UniFFI's cleaner frees it.
    private fun clearSas() { sas?.cancel(); sas = null }

    fun setClipboardEnabled(value: Boolean) { core?.setClipboardEnabled(value) }
    fun sendClipboard() { viewModelScope.launch { core?.sendCurrentClipboard() } }

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
                mutableUi.update { it.copy(screen = "paired", name = peer.name, computerName = peer.name, fingerprint = peer.fingerprint, hasPairedDevice = true, message = "Sparowano · Łączenie z komputerem…") }
                try {
                    owner.connectPaired(peer)
                    mutableUi.update { it.copy(message = "Połączono z komputerem") }
                } catch (error: Exception) {
                    if (error is CancellationException && error !is kotlinx.coroutines.TimeoutCancellationException) throw error
                    mutableUi.update { it.copy(message = "Sparowano. Komputer jest chwilowo nieosiągalny — sprawdź wspólną sieć Wi-Fi.") }
                }
                ConnectionService.start(getApplication(), enable = true)
            } catch (error: Exception) {
                if (error is CancellationException) throw error
                mutableUi.update { it.copy(screen = "error", message = "Nie udało się sparować. Sprawdź wspólną sieć Wi-Fi i zeskanuj nowy kod na komputerze.") }
            } finally { active.destroy() }
        }
    }

    private fun clearOffer() { offer?.destroy(); offer = null }
    override fun onCleared() {
        clearOffer(); clearSas(); stopNearby()
        val owner = core
        if (owner != null) CoroutineScope(Dispatchers.IO).launch { (getApplication<Application>() as WindApplication).connections.release(owner) }
    }
}
