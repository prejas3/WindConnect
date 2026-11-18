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
    val askSend = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { result ->
        val ok = result[Manifest.permission.SEND_SMS] == true
        sendRefused = !ok
        connections.refreshSms()
        if (ok) connections.setSmsSendAllowed(true)
    }
    val turnSend = { on: Boolean ->
        if (on && !sendAccess) askSend.launch(arrayOf(Manifest.permission.SEND_SMS, Manifest.permission.READ_PHONE_STATE))
        else connections.setSmsSendAllowed(on)
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
        actions = if (refused || sendRefused || noAccess || (allowed && access && !contacts)) { {
            WindButton(str(R.string.open_app_settings), {
                val intent = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null))
                if (context !is Activity) intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                context.startActivity(intent)
            }, Modifier.testTag("smsSettings"), kind = ButtonKind.Secondary)
        } } else null,
        more = { Text(str(R.string.sms_about)); Text(str(R.string.sms_permissions)) })
    // WN-56: sending is a separate consent and needs reading on (the PC sends from the window that shows the threads).
    if (allowed && access) {
        WindDivider()
        WindSettingRow("message", str(R.string.sms_send_title), str(R.string.sum_sms_send), checked = sendAllowed && sendAccess, onCheckedChange = turnSend,
            switchTag = "smsSendAllowed", statusTag = "smsSendState", warning = sendRefused || (sendAllowed && !sendAccess),
            status = when {
                sendAllowed && sendAccess -> str(R.string.sms_send_about)
                sendRefused || (sendAllowed && !sendAccess) -> str(R.string.sms_send_denied)
                else -> str(R.string.sms_send_permissions)
            })
    }
}
