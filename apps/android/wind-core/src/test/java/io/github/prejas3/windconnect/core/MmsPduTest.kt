package io.github.prejas3.windconnect.core

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayOutputStream

class MmsPduTest {
    private fun ByteArray.indexOf(sub: ByteArray): Int =
        (0..size - sub.size).firstOrNull { i -> sub.indices.all { this[i + it] == sub[it] } } ?: -1

    @Test fun headersComeFirstInOrder() {
        val pdu = MmsPdu.sendRequest(listOf("+48 600-100-200"), "Hej", null, "T1")
        // m-send-req, transaction "T1", version 1.2, From insert-address-token.
        assertArrayEquals(byteArrayOf(0x8C.toByte(), 0x80.toByte(), 0x98.toByte(), 'T'.code.toByte(), '1'.code.toByte(), 0,
            0x8D.toByte(), 0x92.toByte(), 0x89.toByte(), 1, 0x81.toByte()), pdu.copyOfRange(0, 11))
        assertEquals(0x97, pdu[11].toInt() and 0xFF)
        assertTrue(pdu.indexOf("+48600100200/TYPE=PLMN\u0000".toByteArray()) == 12)
        // Content-Type multipart/related with type and start.
        val ct = pdu.indexOf(byteArrayOf(0x84.toByte()))
        assertEquals(0xB3, pdu[ct + 2].toInt() and 0xFF)
    }

    @Test fun partsCarryTextAndPicture() {
        val jpeg = byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte(), 1, 2, 3)
        val pdu = MmsPdu.sendRequest(listOf("1", "2"), "Zażółć", MmsPdu.Image("image/jpeg", jpeg), "T2") // i18n: data
        assertEquals(2, Regex("/TYPE=PLMN").findAll(String(pdu, Charsets.ISO_8859_1)).count())
        assertTrue(pdu.indexOf(jpeg) > 0)
        assertTrue(pdu.indexOf("Zażółć".toByteArray()) > 0) // i18n: data
        // Text part: value-length 3, text/plain, charset=utf-8.
        assertTrue(pdu.indexOf(byteArrayOf(3, 0x83.toByte(), 0x81.toByte(), 0xEA.toByte())) > 0)
        assertTrue(pdu.indexOf("<img src=\"image.jpg\"".toByteArray()) > 0)
    }

    @Test fun addressesAndUintvar() {
        assertEquals("ola@example.com", MmsPdu.address("ola@example.com"))
        assertEquals("+48600/TYPE=PLMN", MmsPdu.address("+48 600"))
        val out = ByteArrayOutputStream()
        MmsPdu.uintvar(out, 0x7F); MmsPdu.uintvar(out, 0x80); MmsPdu.uintvar(out, 300)
        assertArrayEquals(byteArrayOf(0x7F, 0x81.toByte(), 0x00, 0x82.toByte(), 0x2C), out.toByteArray())
    }

    @Test fun deliveryStatuses() {
        assertEquals(0, SmsRules.delivery(0))
        assertEquals(1, SmsRules.delivery(32))
        assertEquals(2, SmsRules.delivery(64))
        assertEquals(null, SmsRules.providerDelivery(-1))
        assertEquals(1.toUByte(), SmsRules.providerDelivery(0))
        assertEquals(0.toUByte(), SmsRules.providerDelivery(32))
        assertEquals(2.toUByte(), SmsRules.providerDelivery(64))
    }
}
