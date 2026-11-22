package io.github.prejas3.windconnect

import org.junit.Assert.*
import org.junit.Test

class RelaySettingTest {
    init { StringsForTests.useEnglish() }
    @Test fun refusedUrlBecomesTheFieldErrorAndOffAlwaysPasses() {
        val calls = mutableListOf<String?>()
        val set: (String?) -> Unit = { url -> calls += url; if (url?.startsWith("wss://") == false) throw IllegalArgumentException("must start with wss://") }
        assertNull(RelaySetting.apply(set, "wss://windconnect-relay.example.workers.dev"))
        assertEquals("Invalid address: must start with wss://", RelaySetting.apply(set, "http://x"))
        assertEquals("Enter the relay's address", RelaySetting.apply(set, " "))
        assertNull(RelaySetting.apply(set, null))
        assertEquals("WindConnect is still starting",
            RelaySetting.apply({ error("WindConnect is still starting") }, "wss://windconnect-relay.example.workers.dev"))
        assertEquals(listOf("wss://windconnect-relay.example.workers.dev", "http://x", null), calls)
    }
}
