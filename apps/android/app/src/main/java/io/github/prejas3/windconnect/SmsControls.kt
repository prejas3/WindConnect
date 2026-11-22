package io.github.prejas3.windconnect

import io.github.prejas3.windconnect.ui.*
import androidx.compose.ui.semantics.heading

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
    val sendAllowed by connections.smsSendAllowed.collectAsStateWithLifecycle()
    val sendAccess by connections.smsSendAccess.collectAsStateWithLifecycle()
    var sendRefused by rememberSaveable { mutableStateOf(false) }
    var sendBlocked by rememberSaveable { mutableStateOf(false) }
    var askedAt by rememberSaveable { mutableStateOf(0L) }
    // Optional, after SEND_SMS: the SIM list. A refusal changes nothing (one SIM is used).
    val askPhoneState = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { connections.refreshSms() }
    val askSend = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { ok ->
        val rationale = (context as? Activity)?.shouldShowRequestPermissionRationale(Manifest.permission.SEND_SMS) == true
        val outcome = SmsSendAsk.outcome(ok, rationale, android.os.SystemClock.elapsedRealtime() - askedAt)
        sendRefused = outcome == SmsSendAsk.Outcome.Refused
        sendBlocked = outcome == SmsSendAsk.Outcome.Blocked
        connections.refreshSms()
        if (ok) {
            connections.setSmsSendAllowed(true)
            if (!context.granted(Manifest.permission.READ_PHONE_STATE)) askPhoneState.launch(Manifest.permission.READ_PHONE_STATE)
        }
    }
    val turnSend = { on: Boolean ->
        if (SmsSendAsk.onTurn(on, sendAccess) == SmsSendAsk.Step.AskSystem) {
            askedAt = android.os.SystemClock.elapsedRealtime()
            askSend.launch(Manifest.permission.SEND_SMS)
        } else connections.setSmsSendAllowed(on)
    }
    // WN-158: a refusal is only remembered until the access is there (granted later in the app's system settings);
    // otherwise "no consent" stayed red over a granted permission.
    androidx.compose.runtime.LaunchedEffect(sendAccess) { if (sendAccess) { sendRefused = false; sendBlocked = false } }
    androidx.compose.runtime.LaunchedEffect(access) { if (access) refused = false }
    // WN-158: the "app settings" button sits in the row it is about (a send refusal showed it under reading).
    val appSettings: @Composable (String) -> Unit = { tag ->
        WindButton(str(R.string.open_app_settings), {
            val intent = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null))
            if (context !is Activity) intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            context.startActivity(intent)
        }, Modifier.testTag(tag), kind = ButtonKind.Secondary)
    }
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
    // WN-130: one row; the state line only when it says more than the switch (on, or access missing).
    val noAccess = refused || (allowed && !access)
    val state = when {
        allowed && access && contacts -> str(R.string.sms_with_contacts)
        allowed && access -> str(R.string.sms_no_contacts)
        noAccess -> str(R.string.sms_no_access)
        else -> null
    }
    WindSettingRow("message", str(R.string.sms_title), str(R.string.sum_sms), checked = allowed && access, onCheckedChange = turn, switchTag = "smsAllowed",
        status = state, statusTag = "smsState", warning = noAccess,
        actions = if (noAccess || (allowed && access && !contacts)) { { appSettings("smsSettings") } } else null,
        more = { Text(str(R.string.sms_about)); Text(str(R.string.sms_permissions)) })
    // WN-56: sending is a separate consent and needs reading on (the PC sends from the window that shows the threads).
    if (allowed && access) {
        WindDivider()
        WindSettingRow("message", str(R.string.sms_send_title), str(R.string.sum_sms_send), checked = sendAllowed && sendAccess, onCheckedChange = turnSend,
            switchTag = "smsSendAllowed", statusTag = "smsSendState", warning = sendBlocked || sendRefused || (sendAllowed && !sendAccess),
            actions = if (sendBlocked || sendRefused || (sendAllowed && !sendAccess)) { { appSettings("smsSendSettings") } } else null,
            status = when {
                sendAllowed && sendAccess -> str(R.string.sms_send_about)
                // WN-158: Android closed the request without a dialog: say exactly what to change in the settings.
                sendBlocked -> str(R.string.sms_send_blocked)
                sendRefused || (sendAllowed && !sendAccess) -> str(R.string.sms_send_denied)
                else -> str(R.string.sms_send_permissions)
            })
    }
}

private fun android.content.Context.granted(permission: String): Boolean =
    checkSelfPermission(permission) == android.content.pm.PackageManager.PERMISSION_GRANTED

/**
 * WN-158: what turning "Sending from the computer" on does, and what Android's answer means.
 * CCS/1 M:WN-158 cause: on Szymon's S25 (Android 17, One UI 9, APK installed from a file) the request for SEND_SMS
 * reaches GrantPermissionsActivity, which closes in ~20 ms without a dialog and without marking it refused (no
 * USER_SET/USER_FIXED; app-op SEND_SMS stays "ignore"). SEND_SMS is hard-restricted and costs money (flags 0x5);
 * READ_SMS, hard-restricted only, is granted normally, and turning the SMS group off and on in settings didn't help.
 * The app took the system's silent denial for the person's refusal ("no consent, grant it in settings"), while the
 * settings show the SMS group as allowed, so nothing there looked wrong. Now:
 * - turning it on always asks Android first (never settings first), SEND_SMS alone, then READ_PHONE_STATE optionally;
 * - a denial faster than a person can answer, with no rationale to show, is [Outcome.Blocked]: the row says Android
 *   closed the request on this phone (not "you refused") and offers the app's settings in that row.
 */
object SmsSendAsk {
    enum class Step { AskSystem, Save }
    enum class Outcome { Granted, Refused, Blocked }

    /** No dialog can be answered this fast: the system answered for the person. */
    const val NO_DIALOG_MS = 400L

    fun onTurn(on: Boolean, granted: Boolean): Step = if (on && !granted) Step.AskSystem else Step.Save

    fun outcome(granted: Boolean, rationale: Boolean, elapsedMs: Long): Outcome = when {
        granted -> Outcome.Granted
        !rationale && elapsedMs in 0 until NO_DIALOG_MS -> Outcome.Blocked
        else -> Outcome.Refused
    }
}
