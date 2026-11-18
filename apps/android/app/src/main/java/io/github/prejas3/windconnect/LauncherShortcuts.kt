package io.github.prejas3.windconnect

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.ShortcutInfo
import android.content.pm.ShortcutManager
import android.graphics.drawable.Icon

internal enum class LauncherAction(val id: String, val action: String, val label: Int) {
    Clipboard("send_clipboard", "io.github.prejas3.windconnect.shortcut.CLIPBOARD", R.string.shortcut_clipboard),
    File("send_file", "io.github.prejas3.windconnect.shortcut.FILE", R.string.shortcut_file),
    Touchpad("touchpad", "io.github.prejas3.windconnect.shortcut.TOUCHPAD", R.string.shortcut_touchpad);

    /** The shortcut's name in the phone's language (WN-63). */
    val title: String get() = str(label)

    companion object {
        fun fromAction(action: String?) = entries.firstOrNull { it.action == action }
    }
}

internal object LauncherShortcuts {
    fun intent(context: Context, action: LauncherAction) = Intent(context, LauncherShortcutActivity::class.java)
        .setAction(action.action).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK)

    /** CCS/1 RULE:each shortcut opens its real feature screen directly, never Home; the screen itself reports offline/unsupported. */
    fun target(context: Context, action: LauncherAction): Intent = when (action) {
        // SendClipboardActivity has its own empty task affinity, so clearing it never touches the app's task.
        LauncherAction.Clipboard -> Intent(context, SendClipboardActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK)
        LauncherAction.File -> Intent(context, ShareFilesActivity::class.java).setAction(ShareFilesActivity.PICK)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        LauncherAction.Touchpad -> Intent(context, TouchpadActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    }

    fun info(context: Context, action: LauncherAction, rank: Int) = ShortcutInfo.Builder(context, action.id)
        .setActivity(ComponentName(context, MainActivity::class.java))
        .setShortLabel(context.getString(action.label))
        .setLongLabel(action.title)
        .setIcon(Icon.createWithResource(context, R.mipmap.ic_launcher))
        .setIntent(intent(context, action)).setRank(rank).build()

    fun publish(context: Context) {
        // CCS/1 RULE:clipboard is static/immutable; file/touchpad are dynamic. Stable IDs preserve pinned copies,
        // and updateShortcuts refreshes pinned copies made while they were still labelled unavailable.
        val manager = context.getSystemService(ShortcutManager::class.java) ?: return
        try {
            val dynamic = listOf(LauncherAction.File, LauncherAction.Touchpad).mapIndexed { rank, action -> info(context, action, rank) }
            manager.setDynamicShortcuts(dynamic.take((manager.maxShortcutCountPerActivity - 1).coerceAtLeast(0)))
            manager.updateShortcuts(dynamic)
        } catch (_: RuntimeException) {
            // CCS/1 LIMIT:launcher/service restrictions must not prevent app or clipboard startup; retry on next process start.
        }
    }

    fun pin(context: Context, action: LauncherAction): String {
        val manager = context.getSystemService(ShortcutManager::class.java)
        if (manager == null || !manager.isRequestPinShortcutSupported) return str(R.string.shortcut_unsupported)
        return try {
            // CCS/1 RULE:static clipboard is referenced by ID only; it cannot be mutated into a dynamic shortcut.
            val shortcut = if (action == LauncherAction.Clipboard) ShortcutInfo.Builder(context, action.id).build()
                else info(context, action, action.ordinal - 1)
            if (manager.requestPinShortcut(shortcut, null)) str(R.string.shortcut_requested)
            else str(R.string.shortcut_refused)
        } catch (_: RuntimeException) { str(R.string.shortcut_failed) }
    }
}
