package io.github.prejas3.windconnect.core

import java.io.ByteArrayOutputStream

/**
 * CCS/1 M:WN-56 a minimal MMS M-Send.req (OMA MMS 1.2 encapsulation, WAP-230 WSP encoding) for SmsManager.sendMultimediaMessage:
 * To per recipient, From as insert-address-token, multipart/related with a SMIL layout, an optional picture and UTF-8 text.
 * Android has no public PDU composer; this writes only what a send needs. Pure (no Android types), unit-tested.
 */
object MmsPdu {
    class Image(val mime: String, val bytes: ByteArray)

    private const val MESSAGE_TYPE = 0x8C
    private const val SEND_REQ = 0x80
    private const val TRANSACTION_ID = 0x98
    private const val MMS_VERSION = 0x8D
    private const val VERSION_1_2 = 0x92
    private const val FROM = 0x89
    private const val TO = 0x97
    private const val CONTENT_TYPE = 0x84
    private const val INSERT_ADDRESS = 0x81
    private const val MULTIPART_RELATED = 0xB3
    private const val PARAM_TYPE = 0x89
    private const val PARAM_START = 0x8A
    private const val PARAM_CHARSET = 0x81
    private const val UTF8 = 0xEA
    private const val CONTENT_ID = 0xC0
    private const val CONTENT_LOCATION = 0x8E
    private val WELL_KNOWN = mapOf("text/plain" to 0x83, "image/jpeg" to 0x9E, "image/png" to 0xA0)

    /** `to`: numbers (or e-mail addresses), as the person typed them. */
    fun sendRequest(to: List<String>, text: String, image: Image?, transaction: String): ByteArray {
        require(to.isNotEmpty())
        val out = ByteArrayOutputStream()
        out.write(MESSAGE_TYPE); out.write(SEND_REQ)
        out.write(TRANSACTION_ID); textString(out, transaction)
        out.write(MMS_VERSION); out.write(VERSION_1_2)
        out.write(FROM); out.write(1); out.write(INSERT_ADDRESS)
        for (address in to) { out.write(TO); textString(out, address(address)) }
        // Content-Type is the last header: multipart/related; type=application/smil; start=<smil>
        val ct = ByteArrayOutputStream().apply {
            write(MULTIPART_RELATED)
            write(PARAM_TYPE); textString(this, "application/smil")
            write(PARAM_START); textString(this, "<smil>")
        }.toByteArray()
        out.write(CONTENT_TYPE); valueLength(out, ct.size); out.write(ct)
        val imageName = if (image?.mime == "image/png") "image.png" else "image.jpg"
        val parts = buildList {
            add(part("application/smil", "<smil>", "smil.xml", smil(image?.let { imageName }, text.isNotEmpty()).toByteArray(Charsets.UTF_8)))
            if (image != null) add(part(image.mime, "<image>", imageName, image.bytes))
            if (text.isNotEmpty()) add(part("text/plain", "<text>", "text.txt", text.toByteArray(Charsets.UTF_8), utf8 = true))
        }
        uintvar(out, parts.size.toLong())
        for ((headers, data) in parts) {
            uintvar(out, headers.size.toLong()); uintvar(out, data.size.toLong())
            out.write(headers); out.write(data)
        }
        return out.toByteArray()
    }

    /** A phone number gets `/TYPE=PLMN` (spaces and dashes out); an e-mail address stays as it is. */
    fun address(raw: String): String {
        if ('@' in raw) return raw
        val number = raw.filter { it.isDigit() || it == '+' || it == '*' || it == '#' }
        return "$number/TYPE=PLMN"
    }

    fun smil(imageName: String?, hasText: Boolean): String = buildString {
        append("<smil><head><layout><root-layout/>")
        if (imageName != null) append("<region id=\"Image\" fit=\"meet\" top=\"0\" left=\"0\" height=\"80%\" width=\"100%\"/>")
        if (hasText) append("<region id=\"Text\" top=\"80%\" left=\"0\" height=\"20%\" width=\"100%\"/>")
        append("</layout></head><body><par dur=\"5000ms\">")
        if (imageName != null) append("<img src=\"$imageName\" region=\"Image\"/>")
        if (hasText) append("<text src=\"text.txt\" region=\"Text\"/>")
        append("</par></body></smil>")
    }

    /** One part: its encoded content type and headers, and its data. */
    private fun part(mime: String, id: String, location: String, data: ByteArray, utf8: Boolean = false): Pair<ByteArray, ByteArray> {
        val headers = ByteArrayOutputStream()
        val known = WELL_KNOWN[mime]
        if (utf8 && known != null) {
            // Value-length 3: the type, then charset=utf-8.
            headers.write(3); headers.write(known); headers.write(PARAM_CHARSET); headers.write(UTF8)
        } else if (known != null) {
            headers.write(known)
        } else {
            textString(headers, mime)
        }
        headers.write(CONTENT_ID); headers.write('"'.code); headers.write(id.toByteArray(Charsets.US_ASCII)); headers.write(0)
        headers.write(CONTENT_LOCATION); textString(headers, location)
        return headers.toByteArray() to data
    }

    private fun textString(out: ByteArrayOutputStream, text: String) {
        val bytes = text.toByteArray(Charsets.UTF_8)
        if (bytes.isNotEmpty() && bytes[0].toInt() and 0xFF >= 0x80) out.write(0x7F) // quote (WAP-230 8.4.2.1)
        out.write(bytes); out.write(0)
    }

    private fun valueLength(out: ByteArrayOutputStream, length: Int) {
        if (length <= 30) out.write(length) else { out.write(0x1F); uintvar(out, length.toLong()) }
    }

    fun uintvar(out: ByteArrayOutputStream, value: Long) {
        var v = value
        val groups = ArrayList<Int>()
        do { groups.add((v and 0x7F).toInt()); v = v ushr 7 } while (v != 0L)
        for (i in groups.indices.reversed()) out.write(if (i == 0) groups[i] else groups[i] or 0x80)
    }
}
