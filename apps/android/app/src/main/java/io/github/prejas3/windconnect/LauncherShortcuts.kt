package io.github.prejas3.windconnect

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.ShortcutInfo
import android.content.pm.ShortcutManager
import android.graphics.drawable.Icon

internal enum class LauncherAction(val id: String, val action: String, val label: Int, val title: String) {
    Clipboard("send_clipboard", "io.github.prejas3.windconnect.shortcut.CLIPBOARD", R.string.shortcut_clipboard, "Wyślij schowek"),
    File("send_file", "io.github.prejas3.windconnect.shortcut.FILE", R.string.shortcut_file, "Wyślij plik"),
    Touchpad("touchpad", "io.github.prejas3.windconnect.shortcut.TOUCHPAD", R.string.shortcut_touchpad, "Touchpad");

    companion object {
        fun fromAction(action: String?) = entries.firstOrNull { it.action == action }
    }
}

internal object LauncherShortcuts {
    fun intent(context: Context, action: LauncherAction) = Intent(context, LauncherShortcutActivity::class.java)
        .setAction(action.action).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK)

    fun info(context: Context, action: LauncherAction, rank: Int) = ShortcutInfo.Builder(context, action.id)
        .setActivity(ComponentName(context, MainActivity::class.java))
        .setShortLabel(context.getString(action.label))
        .setLongLabel(if (action == LauncherAction.Clipboard) action.title else "Niedostępne · ${action.title}")
        .setIcon(Icon.createWithResource(context, R.mipmap.ic_launcher))
        .setIntent(intent(context, action)).setRank(rank).build()

    fun publish(context: Context) {
        // CCS/1 RULE:clipboard is static/immutable; only future-feature shortcuts are dynamic. Stable IDs preserve pinned copies.
        val manager = context.getSystemService(ShortcutManager::class.java) ?: return
        try {
            manager.setDynamicShortcuts(listOf(LauncherAction.File, LauncherAction.Touchpad)
                .take((manager.maxShortcutCountPerActivity - 1).coerceAtLeast(0))
                .mapIndexed { rank, action -> info(context, action, rank) })
        } catch (_: RuntimeException) {
            // CCS/1 LIMIT:launcher/service restrictions must not prevent app or clipboard startup; retry on next process start.
        }
    }

    fun pin(context: Context, action: LauncherAction): String {
        val manager = context.getSystemService(ShortcutManager::class.java)
        if (manager == null || !manager.isRequestPinShortcutSupported) return "Ten launcher nie obsługuje przypinania skrótów."
        return try {
            // CCS/1 RULE:static clipboard is referenced by ID only; it cannot be mutated into a dynamic shortcut.
            val shortcut = if (action == LauncherAction.Clipboard) ShortcutInfo.Builder(context, action.id).build()
                else info(context, action, action.ordinal - 1)
            if (manager.requestPinShortcut(shortcut, null)) "Prośba o przypięcie została przekazana do launchera. Potwierdź ją, jeśli pojawi się okno."
            else "Launcher nie przyjął prośby o przypięcie skrótu."
        } catch (_: RuntimeException) { "Nie udało się przypiąć skrótu. Spróbuj ponownie w launcherze." }
    }
}
