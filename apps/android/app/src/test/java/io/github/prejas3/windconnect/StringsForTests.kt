package io.github.prejas3.windconnect

import java.io.File
import java.util.Locale
import javax.xml.parsers.DocumentBuilderFactory
import org.w3c.dom.Element

/**
 * WN-63: JVM tests have no Android resources, so [str] reads the app's strings from res/values (English) or
 * res/values-pl (Polish, English for a missing key) here. WN-174: tests run in English, the source language
 * ([useEnglish] in each test's init); [usePolish] is for the checks of the Polish texts.
 */
internal object StringsForTests {
    private fun unescape(raw: String): String {
        val out = StringBuilder()
        var i = 0
        while (i < raw.length) {
            val c = raw[i]
            if (c == '\\' && i + 1 < raw.length) {
                out.append(when (val n = raw[i + 1]) { 'n' -> '\n'; 't' -> '\t'; else -> n }); i += 2
            } else { out.append(c); i++ }
        }
        return out.toString()
    }

    private fun read(folder: String): Pair<Map<String, String>, Map<String, Map<String, String>>> {
        val strings = mutableMapOf<String, String>()
        val plurals = mutableMapOf<String, Map<String, String>>()
        val doc = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(File("src/main/res/$folder/strings.xml"))
        val english = if (folder != "values") read("values") else null
        english?.let { strings.putAll(it.first); plurals.putAll(it.second) }
        val s = doc.getElementsByTagName("string")
        for (i in 0 until s.length) { val e = s.item(i) as Element; strings[e.getAttribute("name")] = unescape(e.textContent) }
        val p = doc.getElementsByTagName("plurals")
        for (i in 0 until p.length) {
            val e = p.item(i) as Element
            val items = e.getElementsByTagName("item")
            plurals[e.getAttribute("name")] = (0 until items.length).associate { (items.item(it) as Element).let { item -> item.getAttribute("quantity") to unescape(item.textContent) } }
        }
        return strings to plurals
    }

    private fun names(kind: String): Map<Int, String> =
        Class.forName("io.github.prejas3.windconnect.R\$$kind").fields.associate { it.getInt(null) to it.name }

    /** Polish, as the app shows it on a Polish phone. */
    fun usePolish() = use("values-pl", Locale.forLanguageTag("pl-PL"))
    fun useEnglish() = use("values", Locale.ENGLISH)

    private fun use(folder: String, locale: Locale) {
        val (strings, plurals) = read(folder)
        val stringNames = names("string"); val pluralNames = names("plurals")
        Strings.lookup = { id, args -> String.format(locale, checkNotNull(strings[stringNames[id]]) { "no string $id" }, *args) }
        Strings.pluralLookup = { id, count, args ->
            val forms = checkNotNull(plurals[pluralNames[id]])
            val quantity = if (locale.language == "pl") when {
                count == 1 -> "one"
                count % 10 in 2..4 && count % 100 !in 12..14 -> "few"
                else -> "many"
            } else if (count == 1) "one" else "other"
            String.format(locale, forms[quantity] ?: forms.getValue("other"), *args)
        }
    }
}
