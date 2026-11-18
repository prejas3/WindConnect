package io.github.prejas3.windconnect

import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.w3c.dom.Element

/** WN-63: every text has an English (values) and a Polish (values-pl) version with the same placeholders. */
class TranslationParityTest {
    private fun entries(file: File): Map<String, List<String>> {
        val doc = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(file)
        val out = mutableMapOf<String, List<String>>()
        val strings = doc.getElementsByTagName("string")
        for (i in 0 until strings.length) {
            val e = strings.item(i) as Element
            if (e.getAttribute("translatable") != "false") out[e.getAttribute("name")] = listOf(e.textContent)
        }
        val plurals = doc.getElementsByTagName("plurals")
        for (i in 0 until plurals.length) {
            val e = plurals.item(i) as Element
            val items = e.getElementsByTagName("item")
            out["plurals:" + e.getAttribute("name")] = (0 until items.length).map { items.item(it).textContent }
        }
        return out
    }

    private fun placeholders(text: String) = Regex("%(\\d+)\\$[sd]").findAll(text).map { it.value }.toSortedSet()

    private fun check(res: String) {
        val english = entries(File("$res/values/strings.xml"))
        val polish = entries(File("$res/values-pl/strings.xml"))
        assertEquals("$res: the same names in values and values-pl", english.keys, polish.keys)
        assertTrue("$res has texts", english.size > 10)
        for ((name, texts) in english) {
            // A plural's forms together use the same placeholders in both languages.
            assertEquals("$res $name: placeholders", texts.flatMap(::placeholders).toSortedSet(),
                polish.getValue(name).flatMap(::placeholders).toSortedSet())
        }
    }

    @Test fun appStringsHaveBothLanguages() = check("src/main/res")
    @Test fun coreStringsHaveBothLanguages() = check("../wind-core/src/main/res")
}
