package io.github.prejas3.windconnect

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class PcActionTextTest {
    init { StringsForTests.usePolish() }
    @Test fun findsTheFirstWebLink() {
        assertEquals("https://a.pl/x?y=1", PcActionText.link("Zobacz: https://a.pl/x?y=1."))
        assertEquals("http://b.com", PcActionText.link("(http://b.com) i https://c.com"))
        assertNull(PcActionText.link("file:///etc/hosts"))
        assertNull(PcActionText.link("brak linku"))
        assertNull(PcActionText.link(null))
    }

    @Test fun everyStatusSaysSomething() {
        for (s in listOf("done", "refused", "bad-argument", "busy", "failed", "unavailable", "weird")) {
            assertTrue(PcActionText.of(s, "Zablokuj PC").isNotBlank())
        }
        assertTrue(PcActionText.of("refused", "Zablokuj PC").contains("odrzucono"))
    }

    /** WN-165: built-ins by id in the phone's language; a custom or unknown action keeps the PC's label. */
    @Test fun actionNamesFollowThePhonesLanguage() {
        try {
            StringsForTests.useEnglish()
            assertEquals("Lock PC", PcActionText.label("lock", "Zablokuj"))
            assertEquals("Sleep PC", PcActionText.label("sleep", "Uśpij"))
            assertEquals("Open on PC", PcActionText.label("open-url", "Otwórz link"))
            StringsForTests.usePolish()
            assertEquals("Zablokuj PC", PcActionText.label("lock", "Lock"))
            assertEquals("Uśpij PC", PcActionText.label("sleep", "Sleep"))
            assertEquals("Otwórz na PC", PcActionText.label("open-url", "Open link"))
            // A custom action is named by the person on the PC; an id from a newer PC shows the PC's own label.
            assertEquals("Kopia zapasowa", PcActionText.label("x-backup", "Kopia zapasowa"))
            assertEquals("Hibernate", PcActionText.label("hibernate", " Hibernate "))
            assertEquals("restart", PcActionText.label("restart", "  "))
        } finally { StringsForTests.usePolish() }
    }
}
