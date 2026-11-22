package io.github.prejas3.windconnect

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class PcActionTextTest {
    init { StringsForTests.useEnglish() }
    @Test fun findsTheFirstWebLink() {
        assertEquals("https://a.pl/x?y=1", PcActionText.link("Zobacz: https://a.pl/x?y=1."))
        assertEquals("http://b.com", PcActionText.link("(http://b.com) i https://c.com"))
        assertNull(PcActionText.link("file:///etc/hosts"))
        assertNull(PcActionText.link("brak linku"))
        assertNull(PcActionText.link(null))
    }

    @Test fun everyStatusSaysSomething() {
        for (s in listOf("done", "refused", "bad-argument", "busy", "failed", "unavailable", "weird")) {
            assertTrue(PcActionText.of(s, "Lock PC").isNotBlank())
        }
        assertTrue(PcActionText.of("refused", "Lock PC").contains("refused"))
    }

    /** WN-165: built-ins by id in the phone's language; a custom or unknown action keeps the PC's label. */
    @Test fun actionNamesFollowThePhonesLanguage() {
        try {
            // The PC's own label (here in another language) never wins over the phone's name for a known id.
            assertEquals("Lock PC", PcActionText.label("lock", "Zablokuj"))
            assertEquals("Sleep PC", PcActionText.label("sleep", "Uśpij")) // i18n: pl
            assertEquals("Open on PC", PcActionText.label("open-url", "Otwórz link")) // i18n: pl
            StringsForTests.usePolish()
            assertEquals("Zablokuj PC", PcActionText.label("lock", "Lock"))
            assertEquals("Uśpij PC", PcActionText.label("sleep", "Sleep")) // i18n: pl
            assertEquals("Otwórz na PC", PcActionText.label("open-url", "Open link")) // i18n: pl
            // A custom action is named by the person on the PC; an id from a newer PC shows the PC's own label.
            assertEquals("Backup", PcActionText.label("x-backup", "Backup"))
            assertEquals("Hibernate", PcActionText.label("hibernate", " Hibernate "))
            assertEquals("restart", PcActionText.label("restart", "  "))
        } finally { StringsForTests.useEnglish() }
    }

    /**
     * WN-173: what the phone sends passes the PC's wind-core `check_url` (printable ASCII, none of "<>\^`{|}, an ASCII
     * host). Before, a link copied as text went as it was (Polish letters, an IDN host, a `|`) and the PC answered bad-argument.
     */
    @Test fun linksAreSentAsThePcTakesThem() {
        fun pcTakes(url: String) = url.all { it.code in 0x21..0x7e && it !in "\"<>\\^`{|}" } &&
            Regex("""^https?://[A-Za-z0-9\-._]+(:\d{1,5})?([/?#].*)?$""", RegexOption.IGNORE_CASE).matches(url)
        val cases = mapOf(
            "https://pl.wikipedia.org/wiki/Łódź" to /* i18n: data */ "https://pl.wikipedia.org/wiki/%C5%81%C3%B3d%C5%BA",
            "https://łódź.pl/" to /* i18n: data */ "https://xn--d-uga0v4h.pl/",
            "https://zażółć.pl:8443/a b?q=x|y{z}" to /* i18n: data */ "https://xn--za-6ja4f8n1l.pl:8443/a%20b?q=x%7Cy%7Bz%7D",
            "HTTPS://a.pl/%C5%81" to "https://a.pl/%C5%81",
            "https://example.com/a?b=1#c" to "https://example.com/a?b=1#c",
            "https://[::1]:443/x" to "https://[::1]:443/x",
        )
        for ((copied, sent) in cases) {
            assertEquals(copied, sent, PcActionText.forPc(copied))
            if (!sent.contains('[')) assertTrue(sent, pcTakes(sent))
        }
        // The link found in copied text, then made ready for the PC: the whole home-tile path.
        assertEquals("https://pl.wikipedia.org/wiki/%C5%81%C3%B3d%C5%BA",
            PcActionText.link("See https://pl.wikipedia.org/wiki/Łódź.")?.let(PcActionText::forPc)) // i18n: data
    }

    /** WN-173: a link the PC couldn't open says so; other actions keep the generic text. */
    @Test fun aLinkThePcCouldNotOpenSaysSo() {
        assertEquals("The computer couldn't open the link in its browser", PcActionText.of("failed", "Open on PC", "open-url"))
        assertEquals("Lock PC: failed", PcActionText.of("failed", "Lock PC", "lock"))
        assertTrue(PcActionText.of("bad-argument", "Open on PC", "open-url").contains("http"))
    }
}
