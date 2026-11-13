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
 * WN-55 Settings: SMS read on the PC (PROTOCOL.md §9.11). Off by default; READ_SMS (and optionally READ_CONTACTS) is asked
 * only when the person turns it on, after this card says what goes where. A refusal leaves the switch off and the rest of
 * the app as it was. Decision D2: only the GitHub/F-Droid build has this.
 */
@Composable
fun SmsControls() {
    val context = LocalContext.current
    val connections = (context.applicationContext as WindApplication).connections
    val allowed by connections.smsAllowed.collectAsStateWithLifecycle()
    val access by connections.smsAccess.collectAsStateWithLifecycle()
    val contacts by connections.smsContacts.collectAsStateWithLifecycle()
    var refused by rememberSaveable { mutableStateOf(false) }
    val lifecycle = LocalLifecycleOwner.current
    DisposableEffect(lifecycle) {
        connections.refreshSms()
        val observer = object : DefaultLifecycleObserver { override fun onResume(owner: LifecycleOwner) = connections.refreshSms() }
        lifecycle.lifecycle.addObserver(observer)
        onDispose { lifecycle.lifecycle.removeObserver(observer) }
    }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { result ->
        val sms = result[Manifest.permission.READ_SMS] == true
        refused = !sms
        connections.refreshSms()
        if (sms) connections.setSmsAllowed(true)
    }
    val turn = { on: Boolean ->
        if (on && !access) {
            ask.launch(arrayOf(Manifest.permission.READ_SMS, Manifest.permission.READ_CONTACTS))
        } else connections.setSmsAllowed(on)
    }
    WindCard {
        Text("SMS na komputerze", style = MaterialTheme.typography.titleMedium)
        Text("Komputer może czytać wątki SMS i MMS z tego telefonu w oknie „Wiadomości”. Treść idzie tylko bezpośrednio na " +
            "sparowany komputer, szyfrowana, i nie jest tam zapisywana na dysku. Kontakty (opcjonalnie) dają nazwy i zdjęcia " +
            "zamiast samych numerów.")
        Text("Po włączeniu Android zapyta o dostęp do SMS i kontaktów. Odmowa niczego innego nie wyłącza.",
            style = MaterialTheme.typography.bodySmall)
        WindSwitch("SMS na komputerze", allowed && access, turn, Modifier.testTag("smsAllowed"))
        val state = when {
            allowed && access && contacts -> "Komputer widzi wątki z nazwami kontaktów."
            allowed && access -> "Komputer widzi wątki; bez dostępu do kontaktów pokaże same numery."
            refused || (allowed && !access) -> "Brak dostępu do SMS. Możesz go przyznać w ustawieniach aplikacji."
            else -> "Wyłączone."
        }
        Text(state, style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.testTag("smsState").semantics { liveRegion = LiveRegionMode.Polite })
        if (refused || (allowed && !access) || (allowed && access && !contacts)) {
            WindButton("Otwórz ustawienia aplikacji", {
                val intent = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null))
                if (context !is Activity) intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                context.startActivity(intent)
            }, Modifier.testTag("smsSettings"), kind = ButtonKind.Secondary)
        }
    }
}
