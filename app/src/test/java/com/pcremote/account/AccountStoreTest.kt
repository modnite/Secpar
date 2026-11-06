package com.pcremote.account

import com.pcremote.agent.KeyValue
import com.pcremote.agent.PairedPc
import com.pcremote.agent.PcStore
import com.pcremote.agent.RelayInfo
import com.pcremote.agent.SecretBox
import org.junit.Assert.*
import org.junit.Test
import java.util.Base64

class AccountStoreTest {
    private class Memory : KeyValue {
        val values = mutableMapOf<String, String>()
        override fun get(key: String) = values[key]
        override fun put(key: String, value: String?) { if (value == null) values.remove(key) else values[key] = value }
    }
    // Deterministic reversible substitute only; production uses authenticated Android Keystore encryption.
    private object Box : SecretBox {
        override fun seal(plain: String) = Base64.getEncoder().encodeToString(plain.toByteArray(Charsets.UTF_8))
        override fun open(sealed: String) = String(Base64.getDecoder().decode(sealed), Charsets.UTF_8)
    }
    private val ownerA = "11111111-1111-4111-8111-111111111111"
    private val ownerB = "22222222-2222-4222-8222-222222222222"
    private val computer = "33333333-3333-4333-8333-333333333333"
    private fun session(owner: String = ownerA, token: String = "a".repeat(64), expires: Long = 2000) =
        AccountSession(AccountUser(owner, "test-user"), token, expires)
    private fun access() = OwnedAccess(computer, PairedPc("host-id", "Private PC", listOf("192.168.1.2"),
        47999, "f".repeat(64), "private-device-token", listOf("00:11:22:33:44:55"), "example.test",
        RelayInfo("wss://relay.example", "host-id", "device-id", ByteArray(32) { 3 })))

    @Test fun `session and computer credentials are sealed and recover across restart`() {
        val kv = Memory()
        val store = AccountStore(kv, Box) { 1000 }
        val session = session()
        store.saveSession(session)
        assertTrue(store.saveOwned(session, access()))
        val restored = AccountStore(kv, Box) { 1000 }
        assertEquals(session.token, restored.session()!!.token)
        val pc = restored.owned(restored.session()!!).single().pc
        assertEquals("private-device-token", pc.token)
        assertEquals("example.test", pc.publicHost)
        assertArrayEquals(ByteArray(32) { 3 }, pc.relay!!.ticket)
        assertFalse(kv.values.values.any { it.contains(session.token) || it.contains(pc.token) || it.contains(pc.name) })
        assertFalse(session.toString().contains(session.token))
        assertFalse(access().toString().contains(pc.token))
    }

    @Test fun `sign out deletes owned access and keeps manual pairings`() {
        val kv = Memory()
        val manual = PcStore(kv, Box)
        manual.upsert(access().pc.copy(token = "manual-token"))
        val before = kv.values["pcremote_pcs"]
        val store = AccountStore(kv, Box) { 1000 }
        val session = session()
        store.saveSession(session)
        store.saveOwned(session, access())
        assertTrue(store.clearSession(session))
        assertNull(store.session())
        assertTrue(store.owned(session).isEmpty())
        assertFalse(kv.values.keys.any { it.startsWith("secpar.account.owned.") })
        assertEquals(before, kv.values["pcremote_pcs"])
        assertEquals("manual-token", manual.all().single().token)
    }

    @Test fun `late pairing and sign out from old session cannot change new session`() {
        val store = AccountStore(Memory(), Box) { 1000 }
        val old = session()
        val fresh = session(token = "b".repeat(64))
        store.saveSession(old)
        store.saveSession(fresh)
        assertFalse(store.saveOwned(old, access()))
        assertFalse(store.clearSession(old))
        assertFalse(store.forgetOwned(old, computer))
        assertEquals(fresh.token, store.session()!!.token)
        assertTrue(store.saveOwned(fresh, access()))
    }

    @Test fun `switching accounts does not expose or retain previous grants`() {
        val kv = Memory()
        val store = AccountStore(kv, Box) { 1000 }
        val a = session()
        val b = session(ownerB, "b".repeat(64))
        store.saveSession(a)
        store.saveOwned(a, access())
        store.saveSession(b)
        assertTrue(store.owned(a).isEmpty())
        assertTrue(store.owned(b).isEmpty())
        assertFalse(store.saveOwned(a, access()))
        assertFalse(kv.values.keys.any { it.endsWith(ownerA) })
        store.saveSession(a)
        assertTrue(store.owned(a).isEmpty())
    }

    @Test fun `expired session cannot read or commit credentials but can be removed`() {
        var clock = 1000L
        val store = AccountStore(Memory(), Box) { clock }
        val session = session(expires = 1001)
        store.saveSession(session)
        store.saveOwned(session, access())
        clock = 1001
        assertNull(store.session())
        assertTrue(store.owned(session).isEmpty())
        assertFalse(store.saveOwned(session, access()))
        assertTrue(store.clearSession(session))
    }

    @Test fun `corrupt ciphertext fails closed`() {
        val kv = Memory()
        val store = AccountStore(kv, Box) { 1000 }
        val session = session()
        store.saveSession(session)
        store.saveOwned(session, access())
        kv.values["secpar.account.owned.$ownerA"] = "invalid!"
        assertTrue(store.owned(session).isEmpty())
        kv.values["secpar.account.session"] = "invalid!"
        assertNull(store.session())
        assertFalse(store.saveOwned(session, access()))
    }

    @Test fun `computer updates replace only the matching cloud computer`() {
        val store = AccountStore(Memory(), Box) { 1000 }
        val session = session()
        store.saveSession(session)
        store.saveOwned(session, access())
        store.saveOwned(session, OwnedAccess("44444444-4444-4444-8444-444444444444", access().pc.copy(id = "other-host")))
        store.saveOwned(session, OwnedAccess(computer, access().pc.copy(name = "Renamed")))
        assertEquals(2, store.owned(session).size)
        assertEquals("Renamed", store.owned(session).first { it.computerId == computer }.pc.name)
        assertTrue(store.forgetOwned(session, computer))
        assertEquals("other-host", store.owned(session).single().pc.id)
    }

    @Test fun `encrypted grants cannot be relocated to another owner or a new session`() {
        val kv = Memory()
        val store = AccountStore(kv, Box) { 1000 }
        val a = session()
        store.saveSession(a)
        store.saveOwned(a, access())
        val oldBlob = kv.values["secpar.account.owned.$ownerA"]!!
        val b = session(ownerB, "b".repeat(64))
        store.saveSession(b)
        kv.values["secpar.account.owned.$ownerB"] = oldBlob
        assertTrue(store.owned(b).isEmpty())
        val freshA = session(token = "c".repeat(64))
        store.saveSession(freshA)
        kv.values["secpar.account.owned.$ownerA"] = oldBlob
        assertTrue(store.owned(freshA).isEmpty())
        assertTrue(store.saveOwned(freshA, access()))
        assertEquals(1, store.owned(freshA).size)
    }
}
