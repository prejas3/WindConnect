package io.github.prejas3.windconnect

import androidx.compose.ui.res.stringResource
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
import io.github.prejas3.windconnect.ui.LocalReduceMotion
import io.github.prejas3.windconnect.ui.WindIcon
import io.github.prejas3.windconnect.ui.WindMotion
import io.github.prejas3.windconnect.ui.WindShapes
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.graphicsLayer
import io.github.prejas3.windconnect.ui.WindButton
import io.github.prejas3.windconnect.ui.WindCard

/**
 * WC-34, first run without a paired computer: what is needed, one main action, and why the camera
 * (said here, before the system asks). The button text matches the computer's step 2.
 */
@Composable
fun WelcomeCard(ready: Boolean, onScan: () -> Unit, onNearby: (() -> Unit)?) {
    WindCard(Modifier.fillMaxWidth().testTag("welcome")) {
        Text(stringResource(R.string.welcome_title), style = MaterialTheme.typography.titleLarge, modifier = Modifier.semantics { heading() })
        Text(stringResource(R.string.welcome_about),
            style = MaterialTheme.typography.bodyMedium)
        WindButton(stringResource(R.string.welcome_scan), onScan, Modifier.fillMaxWidth().testTag("scanQr"), enabled = ready)
        Text(stringResource(R.string.welcome_camera), style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.testTag("cameraNote"))
        // WN-27: pairing by code comparison when the camera can't be used.
        if (onNearby != null) WindButton(stringResource(R.string.pair_without_camera), onNearby, Modifier.fillMaxWidth().testTag("pairNearby"), enabled = ready, kind = ButtonKind.Secondary)
    }
}

/** What the person decided on the stringResource(R.string.done) screen; [notificationsDenied] only after "Utrzymuj". */
data class BackgroundChoice(val decided: Boolean = false, val keep: Boolean = false, val notificationsDenied: Boolean = false)

/** One sentence about the consequence of the background choice (WC-34 review: notifications refused). */
fun backgroundOutcome(choice: BackgroundChoice): String? = when {
    !choice.decided -> null
    !choice.keep -> str(R.string.background_later)
    choice.notificationsDenied -> str(R.string.background_no_notifications)
    else -> str(R.string.background_on)
}

/**
 * WC-34, after pairing: stringResource(R.string.done) and the single decision about the background connection
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
        // WN-171: the hero moment: a large check that grows in (Expressive), then the heading under it, centred.
        DoneMark(Modifier.align(androidx.compose.ui.Alignment.CenterHorizontally).padding(top = 24.dp, bottom = 8.dp))
        Text(stringResource(R.string.done), style = MaterialTheme.typography.headlineMedium, textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            modifier = Modifier.fillMaxWidth().testTag("pairingSuccess").semantics { heading() })
        Text(stringResource(R.string.paired_with, computerLabel(name)), style = MaterialTheme.typography.titleMedium, textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            modifier = Modifier.fillMaxWidth())
        Text(status, modifier = Modifier.testTag("pairedConnectionStatus"))
        Text(stringResource(R.string.paired_key_saved), style = MaterialTheme.typography.bodySmall)
        WindCard(Modifier.fillMaxWidth().testTag("backgroundChoice")) {
            Text(stringResource(R.string.background_title), style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
            Text(stringResource(R.string.background_about),
                style = MaterialTheme.typography.bodyMedium)
            if (!choice.decided) {
                WindButton(stringResource(R.string.background_keep), {
                    ConnectionService.start(context, enable = true)
                    decided = true; keep = true
                    val needs = Build.VERSION.SDK_INT >= 33 &&
                        context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
                    if (needs) permission.launch(Manifest.permission.POST_NOTIFICATIONS)
                }, Modifier.fillMaxWidth().testTag("keepBackground"))
                WindButton(stringResource(R.string.not_now), { decided = true; keep = false }, Modifier.fillMaxWidth().testTag("backgroundLater"), kind = ButtonKind.Quiet)
            } else {
                Text(backgroundOutcome(choice).orEmpty(), modifier = Modifier.testTag("backgroundOutcome"), style = MaterialTheme.typography.bodyMedium)
                if (choice.notificationsDenied) WindButton(stringResource(R.string.notification_settings), {
                    context.startActivity(Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName))
                }, kind = ButtonKind.Quiet)
            }
        }
        WindButton(stringResource(R.string.done), onDone, Modifier.fillMaxWidth().testTag("pairingDone"), kind = if (choice.decided) ButtonKind.Primary else ButtonKind.Secondary)
    }
}

/** WN-171: the "Done" mark: a 96 dp primaryContainer circle with a check, growing in on the slow spatial spring. */
@Composable
private fun DoneMark(modifier: Modifier = Modifier) {
    val colors = MaterialTheme.colorScheme
    val reduce = LocalReduceMotion.current
    val shown = androidx.compose.runtime.remember { androidx.compose.animation.core.Animatable(if (reduce) 1f else 0f) }
    androidx.compose.runtime.LaunchedEffect(Unit) { shown.animateTo(1f, WindMotion.slowSpatial()) }
    androidx.compose.foundation.layout.Box(modifier.size(96.dp).androidxGraphics(shown.value).clip(WindShapes.full).background(colors.primaryContainer).testTag("doneMark"),
        contentAlignment = androidx.compose.ui.Alignment.Center) {
        WindIcon("check", Modifier.size(52.dp), tint = colors.onPrimaryContainer)
    }
}

private fun Modifier.androidxGraphics(progress: Float): Modifier = this.graphicsLayer {
    alpha = progress.coerceIn(0f, 1f)
    val scale = .5f + .5f * progress
    scaleX = scale; scaleY = scale
}
