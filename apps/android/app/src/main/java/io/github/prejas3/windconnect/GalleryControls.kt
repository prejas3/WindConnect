package io.github.prejas3.windconnect

import io.github.prejas3.windconnect.ui.*
import androidx.compose.ui.semantics.heading

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
import io.github.prejas3.windconnect.core.PhoneGallery
import io.github.prejas3.windconnect.ui.ButtonKind
import io.github.prejas3.windconnect.ui.WindButton
import io.github.prejas3.windconnect.ui.WindCard
import io.github.prejas3.windconnect.ui.WindSwitch

/**
 * WN-58 Settings: the newest photos and screenshots in the PC's panel (PROTOCOL.md §9.13). Off by default; photo access is
 * asked only when the person turns it on, after this card says what goes where. A refusal leaves the switch off.
 */
@Composable
fun GalleryControls() {
    val context = LocalContext.current
    val connections = (context.applicationContext as WindApplication).connections
    val allowed by connections.galleryAllowed.collectAsStateWithLifecycle()
    val access by connections.galleryAccess.collectAsStateWithLifecycle()
    var refused by rememberSaveable { mutableStateOf(false) }
    val lifecycle = LocalLifecycleOwner.current
    DisposableEffect(lifecycle) {
        connections.refreshGallery()
        val observer = object : DefaultLifecycleObserver { override fun onResume(owner: LifecycleOwner) = connections.refreshGallery() }
        lifecycle.lifecycle.addObserver(observer)
        onDispose { lifecycle.lifecycle.removeObserver(observer) }
    }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { ok ->
        refused = !ok
        connections.refreshGallery()
        if (ok) connections.setGalleryAllowed(true)
    }
    val turn = { on: Boolean ->
        if (on && !access) ask.launch(PhoneGallery.PERMISSION) else connections.setGalleryAllowed(on)
    }
    val noAccess = refused || (allowed && !access)
    WindSettingRow("image", str(R.string.gallery_title), str(R.string.sum_gallery), checked = allowed && access, onCheckedChange = turn, switchTag = "galleryAllowed",
        status = when { allowed && access -> str(R.string.gallery_on); noAccess -> str(R.string.gallery_no_access); else -> null },
        statusTag = "galleryState", warning = noAccess,
        actions = if (noAccess) { {
            WindButton(str(R.string.open_app_settings), {
                val intent = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null))
                if (context !is Activity) intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                context.startActivity(intent)
            }, Modifier.testTag("gallerySettings"), kind = ButtonKind.Secondary)
        } } else null,
        more = { Text(str(R.string.gallery_about)); Text(str(R.string.gallery_permission)) })
}
