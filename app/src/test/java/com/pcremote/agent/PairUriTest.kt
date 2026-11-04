package com.pcremote.agent

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class PairUriTest {
    private val fp = "a".repeat(64)

    @Test fun `parses the protocol example`() {
        val p = PairUri.parse("pcremote://pair?n=My%20PC&h=192.168.0.126,10.0.0.5&p=47999&fp=$fp&t=abcDEF_-123")!!
        assertEquals("My PC", p.name)
        assertEquals(listOf("192.168.0.126", "10.0.0.5"), p.hosts)
        assertEquals(47999, p.port)
        assertEquals(fp, p.fingerprint)
        assertEquals("abcDEF_-123", p.otp)
    }

    @Test fun `rejects malformed codes`() {
        val ok = "n=PC&h=1.2.3.4&p=47999&fp=$fp&t=x"
        assertNull(PairUri.parse("https://pair?$ok"))
        assertNull(PairUri.parse("pcremote://pair?n=PC&h=1.2.3.4&p=47999&t=x"))
        assertNull(PairUri.parse("pcremote://pair?n=PC&h=1.2.3.4&p=47999&fp=zz&t=x"))
        assertNull(PairUri.parse("pcremote://pair?n=PC&h=&p=47999&fp=$fp&t=x"))
        assertNull(PairUri.parse("pcremote://pair?n=PC&h=1.2.3.4&p=70000&fp=$fp&t=x"))
        assertNull(PairUri.parse("pcremote://pair?n=PC&h=1.2.3.4&p=47999&fp=$fp&t="))
        assertNull(PairUri.parse("garbage"))
    }

    @Test fun `fingerprint is normalised to lowercase`() {
        assertEquals(fp, PairUri.parse("pcremote://pair?n=PC&h=1.2.3.4&p=47999&fp=${fp.uppercase()}&t=x")!!.fingerprint)
    }

    @Test fun `remote code accepts empty LAN hosts and keeps signed URL`() {
        val relay = "wss://relay.example/v1/phone?pc=pc-1&dev=pair_test&ts=123&sig=abc"
        val encoded = java.net.URLEncoder.encode(relay, "UTF-8")
        val p = PairUri.parse("pcremote://pair?n=Desk&h=&p=47999&fp=$fp&t=otp&pc=pc-1&r=$encoded")!!
        assertEquals(relay, p.relayUrl)
        assertEquals("pc-1", p.pcId)
        assertEquals(emptyList<String>(), p.hosts)
    }

    @Test fun `remote code rejects insecure or malformed URL`() {
        for (url in listOf("ws://relay.example/v1/phone", "wss://user@relay.example/v1/phone", "wss://relay.example/wrong")) {
            val encoded = java.net.URLEncoder.encode(url, "UTF-8")
            assertNull(PairUri.parse("pcremote://pair?h=1.2.3.4&p=47999&fp=$fp&t=otp&pc=pc-1&r=$encoded"))
        }
        assertNull(PairUri.parse("pcremote://pair?h=1.2.3.4&p=47999&fp=$fp&t=%ZZ"))
    }

    @Test fun `Secpar scheme supports current host and legacy preview codes`() {
        val code = "n=Secpar&h=192.168.0.9&p=48999&fp=$fp&t=otp"
        assertEquals(PairUri.parse("pcremote://pair?$code"), PairUri.parse("secpar://pair?$code"))
    }
}
