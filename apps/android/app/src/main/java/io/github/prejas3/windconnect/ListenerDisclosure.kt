package io.github.prejas3.windconnect

import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import io.github.prejas3.windconnect.ui.ButtonKind
import io.github.prejas3.windconnect.ui.WindButton
import io.github.prejas3.windconnect.ui.WindDialog

/**
 * WN-179: Google Play's prominent disclosure (User Data policy) before Android's notification access screen. Both
 * notification-listener accesses (notifications, WN-42; the player, WN-46) are reached from the app only through this
 * dialog: it says what leaves the phone and where to, and only "Continue" opens the system screen. "Not now" (or back)
 * opens nothing and changes no setting.
 */
@Composable
fun ListenerDisclosure(icon: String, title: String, text: String, next: String, onContinue: () -> Unit, onDismiss: () -> Unit) {
    WindDialog(onDismiss, title, Modifier.testTag("listenerDisclosure"), icon = icon, actions = {
        WindButton(str(R.string.not_now), onDismiss, Modifier.testTag("disclosureNotNow"), kind = ButtonKind.Quiet)
        WindButton(str(R.string.disclosure_continue), onContinue, Modifier.testTag("disclosureContinue"))
    }) {
        Text(text)
        Text(next)
    }
}
