package io.github.prejas3.windconnect

import org.junit.Assert.*
import org.junit.Test

class LauncherActionTest {
    init { StringsForTests.useEnglish() }
    @Test fun acceptsOnlyTheThreeFixedActions() {
        LauncherAction.entries.forEach { assertEquals(it, LauncherAction.fromAction(it.action)) }
        listOf(null, "", "android.intent.action.VIEW", "io.github.prejas3.windconnect.shortcut.FILE/../../",
            "https://example.org").forEach { assertNull(LauncherAction.fromAction(it)) }
        assertEquals(3, LauncherAction.entries.map { it.id }.toSet().size)
    }
}
