package io.github.prejas3.windconnect

import org.junit.Assert.*
import org.junit.Test

class RelaySettingTest {
    @Test fun refusedUrlBecomesTheFieldErrorAndOffAlwaysPasses() {
        val calls = mutableListOf<String?>()
        val set: (String?) -> Unit = { url -> calls += url; if (url?.startsWith("wss://") == false) throw IllegalArgumentException("must start with wss://") }
        assertNull(RelaySetting.apply(set, "wss://windconnect-relay.example.workers.dev"))
        assertEquals("Nieprawidłowy adres: must start with wss://", RelaySetting.apply(set, "http://x"))
        assertEquals("Podaj adres przekaźnika", RelaySetting.apply(set, " "))
        assertNull(RelaySetting.apply(set, null))
        assertEquals(listOf("wss://windconnect-relay.example.workers.dev", "http://x", null), calls)
    }
}
