package com.pcremote.home

import com.pcremote.agent.KeyValue
import com.pcremote.agent.SecretBox
import org.junit.Assert.*
import org.junit.Test

class RemoteWakeTest {
    private class Memory : KeyValue {
        val data = mutableMapOf<String, String>()
        override fun get(key: String) = data[key]
        override fun put(key: String, value: String?) { if (value == null) data.remove(key) else data[key] = value }
    }
    private val box = object : SecretBox {
        override fun seal(plain: String) = plain.reversed()
        override fun open(sealed: String) = sealed.reversed()
    }
    @Test fun existingHomeAssistantSetupSurvivesNewMethodSelection() {
        val kv = Memory()
        val ha = HomeAssistant(kv, box)
        ha.url = "https://home.example/api/webhook/wake"
        val wake = RemoteWake(kv, box)
        assertEquals(WakeMethod.HOME_ASSISTANT, wake.method)
        wake.method = WakeMethod.LOCAL
        assertEquals(WakeMethod.LOCAL, RemoteWake(kv, box).method)
        assertTrue(ha.configured)
    }
    @Test fun capabilityUrlAndTokenAreEncryptedAtRest() {
        val kv = Memory(); val wake = RemoteWake(kv, box)
        wake.url = "https://wake.example/private-secret"; wake.token = "private-token"
        assertFalse(kv.data.values.any { it.contains("private-secret") || it.contains("private-token") })
        assertEquals("https://wake.example/private-secret", RemoteWake(kv, box).url)
        assertEquals("private-token", RemoteWake(kv, box).token)
    }
    @Test fun webhookRejectsInsecureOrEmbeddedCredentialsAndHasPortablePayload() {
        for (url in listOf("http://wake.example", "https://user:password@wake.example", "https://wake.example/#secret", "invalid"))
            assertFalse(url, RemoteWake.validUrl(url))
        val request = RemoteWake.request("https://wake.example/run", "secret", listOf("AA:BB:CC:DD:EE:FF"))
        assertEquals("POST", request.method)
        assertEquals("Bearer secret", request.header("Authorization"))
        val buffer = okio.Buffer(); request.body!!.writeTo(buffer)
        assertEquals("{\"macs\":[\"AA:BB:CC:DD:EE:FF\"]}", buffer.readUtf8())
    }
}
