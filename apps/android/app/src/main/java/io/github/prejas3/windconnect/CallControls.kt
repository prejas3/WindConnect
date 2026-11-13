package io.github.prejas3.windconnect

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.prejas3.windconnect.ui.ButtonKind
import io.github.prejas3.windconnect.ui.WindButton
import io.github.prejas3.windconnect.ui.WindCard
import io.github.prejas3.windconnect.ui.WindSwitch

/**
 * WN-57 Settings: calls on the PC (PROTOCOL.md §9.10, §9.12). Off by default; phone state and call log (number, missed calls),
 * answering calls (reject from the PC) and contacts (names) are asked only when the person turns it on. Without it the PC still
 * gets the bare call state (WN-48: pausing music), nothing else. A refusal changes nothing else.
 */
@Composable
fun CallControls() {
    val context = LocalContext.current
    val connections = (context.applicationContext as WindApplication).connections
    val allowed by connections.callsAllowed.collectAsStateWithLifecycle()
    val access by connections.callsAccess.collectAsStateWithLifecycle()
    val reject by connections.callsReject.collectAsStateWithLifecycle()
    var refused by rememberSaveable { mutableStateOf(false) }
    val lifecycle = LocalLifecycleOwner.current
    DisposableEffect(lifecycle) {
        connections.refreshCalls()
        val observer = object : DefaultLifecycleObserver { override fun onResume(owner: LifecycleOwner) = connections.refreshCalls() }
        lifecycle.lifecycle.addObserver(observer)
        onDispose { lifecycle.lifecycle.removeObserver(observer) }
    }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { result ->
        val ok = result[Manifest.permission.READ_PHONE_STATE] == true && result[Manifest.permission.READ_CALL_LOG] == true
        refused = !ok
        connections.refreshCalls()
        if (ok) connections.setCallsAllowed(true)
    }
    val turn = { on: Boolean ->
        if (on && !access) ask.launch(arrayOf(Manifest.permission.READ_PHONE_STATE, Manifest.permission.READ_CALL_LOG,
            Manifest.permission.ANSWER_PHONE_CALLS, Manifest.permission.READ_CONTACTS))
        else connections.setCallsAllowed(on)
    }
    WindCard {
        Text("Połączenia na komputerze", style = MaterialTheme.typography.titleMedium)
        Text("Przy połączeniu przychodzącym komputer pokaże powiadomienie z numerem i nazwą kontaktu, z przyciskami „Odrzuć” " +
            "i „Wycisz”, oraz listę nieodebranych połączeń z ostatniego tygodnia. Dane idą tylko na sparowany komputer.")
        Text("Po włączeniu Android zapyta o stan telefonu, rejestr połączeń, odbieranie połączeń (odrzucanie z komputera) " +
            "i kontakty (nazwy). Bez tej funkcji komputer dalej wie tylko, że telefon dzwoni (wstrzymanie muzyki).",
            style = MaterialTheme.typography.bodySmall)
        WindSwitch("Połączenia na komputerze", allowed && access, turn, Modifier.testTag("callsAllowed"))
        Text(when {
            allowed && access && reject -> "Komputer pokaże, kto dzwoni, i może odrzucić albo wyciszyć połączenie."
            allowed && access -> "Komputer pokaże, kto dzwoni, i może wyciszyć dzwonek; bez zgody na odbieranie połączeń nie odrzuci."
            refused || (allowed && !access) -> "Brak dostępu do stanu telefonu lub rejestru połączeń. Możesz go przyznać w ustawieniach aplikacji."
            else -> "Wyłączone."
        }, style = MaterialTheme.typography.bodySmall, modifier = Modifier.testTag("callsState").semantics { liveRegion = LiveRegionMode.Polite })
        if (refused || (allowed && (!access || !reject))) {
            WindButton("Otwórz ustawienia aplikacji", {
                val intent = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null))
                if (context !is Activity) intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                context.startActivity(intent)
            }, Modifier.testTag("callsSettings"), kind = ButtonKind.Secondary)
        }
    }
}
