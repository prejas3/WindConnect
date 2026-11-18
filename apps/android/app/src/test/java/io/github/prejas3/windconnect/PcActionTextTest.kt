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
}
