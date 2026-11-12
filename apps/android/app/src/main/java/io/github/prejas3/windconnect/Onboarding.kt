package io.github.prejas3.windconnect

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import io.github.prejas3.windconnect.ui.ButtonKind
import io.github.prejas3.windconnect.ui.WindButton
import io.github.prejas3.windconnect.ui.WindCard

/**
 * WC-34, first run without a paired computer: what is needed, one main action, and why the camera
 * (said here, before the system asks). The button text matches the computer's step 2.
 */
@Composable
fun WelcomeCard(ready: Boolean, onScan: () -> Unit, onNearby: (() -> Unit)?) {
    WindCard(Modifier.fillMaxWidth().testTag("welcome")) {
        Text("Witaj w WindConnect", style = MaterialTheme.typography.titleLarge, modifier = Modifier.semantics { heading() })
        Text("Połącz telefon z komputerem w kilka sekund. Potrzebujesz komputera z WindConnect w tej samej sieci Wi-Fi. Bez konta i bez chmury: połączenie jest szyfrowane od końca do końca.",
            style = MaterialTheme.typography.bodyMedium)
        WindButton("Zeskanuj kod z komputera", onScan, Modifier.fillMaxWidth().testTag("scanQr"), enabled = ready)
        Text("Aparat działa tylko podczas skanowania, a obraz nie opuszcza telefonu.", style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.testTag("cameraNote"))
        // WN-27: pairing by code comparison when the camera can't be used.
        if (onNearby != null) WindButton("Paruj bez aparatu", onNearby, Modifier.fillMaxWidth().testTag("pairNearby"), enabled = ready, kind = ButtonKind.Secondary)
    }
}

/** What the person decided on the "Gotowe" screen; [notificationsDenied] only after "Utrzymuj". */
data class BackgroundChoice(val decided: Boolean = false, val keep: Boolean = false, val notificationsDenied: Boolean = false)

/** One sentence about the consequence of the background choice (WC-34 review: notifications refused). */
fun backgroundOutcome(choice: BackgroundChoice): String? = when {
    !choice.decided -> null
    !choice.keep -> "Możesz włączyć połączenie w tle później w Ustawieniach."
    choice.notificationsDenied -> "Połączenie działa w tle, ale bez powiadomień nie zobaczysz stanu komputera ani akcji „Wyślij schowek”. Zmienisz to w Ustawieniach."
    else -> "Włączone. Stan komputera i szybkie akcje zobaczysz w powiadomieniu."
}

/**
 * WC-34, after pairing: "Gotowe" and the single decision about the background connection
 * (ConnectionService + notifications). Pairing no longer turns it on by itself.
 */
@Composable
fun DoneScreen(name: String, status: String, onDone: () -> Unit) {
    val context = LocalContext.current
    var decided by rememberSaveable { mutableStateOf(false) }
    var keep by rememberSaveable { mutableStateOf(false) }
    var denied by rememberSaveable { mutableStateOf(false) }
    val choice = BackgroundChoice(decided, keep, denied)
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted -> denied = !granted }
    Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
        Text("Gotowe", style = MaterialTheme.typography.headlineSmall, modifier = Modifier.testTag("pairingSuccess").semantics { heading() })
        Text("Sparowano z: ${computerLabel(name)}", style = MaterialTheme.typography.titleMedium)
        Text(status, modifier = Modifier.testTag("pairedConnectionStatus"))
        Text("Klucz komputera jest bezpiecznie zapamiętany na tym telefonie.", style = MaterialTheme.typography.bodySmall)
        WindCard(Modifier.fillMaxWidth().testTag("backgroundChoice")) {
            Text("Połączenie w tle", style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
            Text("Telefon zostaje połączony z komputerem także po zamknięciu aplikacji. W powiadomieniu zobaczysz stan komputera i akcję „Wyślij schowek”.",
                style = MaterialTheme.typography.bodyMedium)
            if (!choice.decided) {
                WindButton("Utrzymuj połączenie w tle", {
                    ConnectionService.start(context, enable = true)
                    decided = true; keep = true
                    val needs = Build.VERSION.SDK_INT >= 33 &&
                        context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
                    if (needs) permission.launch(Manifest.permission.POST_NOTIFICATIONS)
                }, Modifier.fillMaxWidth().testTag("keepBackground"))
                WindButton("Nie teraz", { decided = true; keep = false }, Modifier.fillMaxWidth().testTag("backgroundLater"), kind = ButtonKind.Quiet)
            } else {
                Text(backgroundOutcome(choice).orEmpty(), modifier = Modifier.testTag("backgroundOutcome"), style = MaterialTheme.typography.bodyMedium)
                if (choice.notificationsDenied) WindButton("Ustawienia powiadomień", {
                    context.startActivity(Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName))
                }, kind = ButtonKind.Quiet)
            }
        }
        WindButton("Gotowe", onDone, Modifier.fillMaxWidth().testTag("pairingDone"), kind = if (choice.decided) ButtonKind.Primary else ButtonKind.Secondary)
    }
}
