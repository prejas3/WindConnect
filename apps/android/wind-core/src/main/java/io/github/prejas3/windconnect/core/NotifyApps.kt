package io.github.prejas3.windconnect.core

import android.content.Context
import io.github.prejas3.windconnect.bindings.PhoneNotification
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** WN-45: which apps' notifications go to the PC. */
enum class NotifyAppMode {
    /** Every app but those switched off (the default, as before WN-45). */
    AllExcept,
    /** Only the apps switched on; new apps are off. */
    OnlyChosen,
}

/**
 * WN-45: the app filter's settings. [seen]: apps the listener saw posting a notification that could go to the PC, newest last,
 * at most [NotifyApps.MAX_SEEN] (package names only, no content). [blocked] counts in [NotifyAppMode.AllExcept], [chosen] in
 * [NotifyAppMode.OnlyChosen]: switching modes keeps both lists.
 */
data class NotifyAppsState(
    val mode: NotifyAppMode = NotifyAppMode.AllExcept,
    val blocked: Set<String> = emptySet(),
    val chosen: Set<String> = emptySet(),
    val seen: List<String> = emptyList(),
) {
    fun allows(packageName: String) = AppFilter.allows(packageName, mode, blocked, chosen)

    /** "Tylko wybrane" with nothing chosen: nothing goes to the PC (the UI says so). */
    val sendsNothing: Boolean get() = mode == NotifyAppMode.OnlyChosen && chosen.isEmpty()
}

/** CCS/1 RULE:WN-45 the filter itself: a blocked app's notification never reaches the core (nothing over the network). */
internal object AppFilter {
    fun allows(packageName: String, mode: NotifyAppMode, blocked: Set<String>, chosen: Set<String>): Boolean = when (mode) {
        NotifyAppMode.AllExcept -> packageName !in blocked
        NotifyAppMode.OnlyChosen -> packageName in chosen
    }
}

/** The filter's settings, next to "Powiadomienia na komputerze"; read by the listener and the core's send loop. */
internal object NotifyApps {
    const val MAX_SEEN = 300
    private const val MODE = "notify_app_mode"
    private const val BLOCKED = "notify_apps_blocked"
    private const val CHOSEN = "notify_apps_chosen"
    private const val SEEN = "notify_apps_seen"

    private val flow = MutableStateFlow<NotifyAppsState?>(null)

    private fun prefs(context: Context) = context.applicationContext.getSharedPreferences(WindCore.SETTINGS_FILE, Context.MODE_PRIVATE)

    private fun list(value: String?) = value?.split('\n')?.filter { it.isNotEmpty() } ?: emptyList()

    @Synchronized fun state(context: Context): NotifyAppsState = flow.value ?: prefs(context).let { p ->
        NotifyAppsState(
            mode = if (p.getString(MODE, null) == "only") NotifyAppMode.OnlyChosen else NotifyAppMode.AllExcept,
            blocked = list(p.getString(BLOCKED, null)).toSet(),
            chosen = list(p.getString(CHOSEN, null)).toSet(),
            seen = list(p.getString(SEEN, null)),
        )
    }.also { flow.value = it }

    fun states(context: Context): StateFlow<NotifyAppsState?> { state(context); return flow.asStateFlow() }

    fun allows(context: Context, packageName: String) = state(context).allows(packageName)

    @Synchronized private fun save(context: Context, next: NotifyAppsState) {
        prefs(context).edit()
            .putString(MODE, if (next.mode == NotifyAppMode.OnlyChosen) "only" else "all")
            .putString(BLOCKED, next.blocked.sorted().joinToString("\n"))
            .putString(CHOSEN, next.chosen.sorted().joinToString("\n"))
            .putString(SEEN, next.seen.joinToString("\n"))
            .apply()
        flow.value = next
    }

    /** The listener saw [packageName] post something that could go to the PC. */
    @Synchronized fun saw(context: Context, packageName: String) {
        val now = state(context)
        if (packageName in now.seen) return
        save(context, now.copy(seen = (now.seen + packageName).takeLast(MAX_SEEN)))
    }

    @Synchronized fun setMode(context: Context, mode: NotifyAppMode) = save(context, state(context).copy(mode = mode))

    /** On or off for the PC, in the current mode. */
    @Synchronized fun set(context: Context, packageName: String, on: Boolean) {
        val now = state(context)
        save(context, when (now.mode) {
            NotifyAppMode.AllExcept -> now.copy(blocked = if (on) now.blocked - packageName else now.blocked + packageName)
            NotifyAppMode.OnlyChosen -> now.copy(chosen = if (on) now.chosen + packageName else now.chosen - packageName)
        })
    }
}

/**
 * CCS/1 FLOW:WN-45 listener events -> [handle] -> the core. RULE:the filter is checked again here, at the send point: a notification
 * queued before its app was blocked is dropped ("filtered"), icon included. Keeps which app each id sent in this session belongs to,
 * so blocking an app sends `Removed` for its notifications still shown on the PC (and forgets them for WN-44's reply/dismiss).
 */
internal class NotifyForwarder(
    private val allows: (String) -> Boolean,
    /** Sends one (the app's label and icon added by the caller); the core's outcome. */
    private val send: suspend (PhoneNotification, String) -> String,
    private val removed: suspend (ByteArray) -> Unit,
    private val forget: (List<ByteArray>) -> Unit,
) {
    private val sent = LinkedHashMap<List<Byte>, String>()

    /** Sent to the PC in this session and not gone since (WN-44: the PC may act only on these). */
    fun isSent(id: ByteArray) = synchronized(sent) { id.toList() in sent }

    /** The outcome for a posted one ("filtered" when its app is now blocked); null for the rest. */
    suspend fun handle(event: NotificationEvent): String? = when (event) {
        is NotificationEvent.Posted -> {
            if (!allows(event.packageName)) "filtered"
            else send(event.notification, event.packageName).also { outcome ->
                if (outcome == "sent" || outcome == "held") synchronized(sent) { sent[event.notification.id.toList()] = event.packageName }
            }
        }
        is NotificationEvent.Removed -> {
            synchronized(sent) { sent.remove(event.id.toList()) }
            removed(event.id)
            null
        }
        NotificationEvent.FilterChanged -> {
            val gone = synchronized(sent) {
                sent.filterValues { !allows(it) }.keys.toList().also { ids -> ids.forEach { sent.remove(it) } }
            }.map { it.toByteArray() }
            forget(gone)
            gone.forEach { removed(it) }
            null
        }
    }
}
