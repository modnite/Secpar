package com.pcremote.account

import com.pcremote.agent.*
import org.junit.Assert.*
import org.junit.Test
import java.util.Base64

class OwnedPcAccessTest {
    private class Memory : KeyValue {
        val values = mutableMapOf<String, String>()
        override fun get(key: String) = values[key]
        override fun put(key: String, value: String?) { if (value == null) values.remove(key) else values[key] = value }
    }
    private object Box : SecretBox {
        override fun seal(plain: String) = Base64.getEncoder().encodeToString(plain.toByteArray())
        override fun open(sealed: String) = String(Base64.getDecoder().decode(sealed))
    }
    private val owner = "11111111-1111-4111-8111-111111111111"
    private val computer = "22222222-2222-4222-8222-222222222222"
    private fun session(token: String = "a".repeat(64)) = AccountSession(AccountUser(owner, "review-user"), token, 2000)
    private val pc get() = PairedPc("host-id", "Review PC", listOf("192.0.2.2"), 47999, "f".repeat(64), "owned-token", emptyList())

    @Test fun `account overlay follows sign in and sign out without creating manual entries`() {
        val kv = Memory()
        val vault = AccountStore(kv, Box) { 1000 }
        val combined = PcStore(kv, Box, OwnedPcAccess(vault))
        val session = session()
        vault.saveSession(session); vault.saveOwned(session, OwnedAccess(computer, pc))
        val owned = combined.all().single()
        assertEquals(owner, owned.accountOwner)
        combined.upsert(owned.copy(hosts = listOf("192.0.2.3")))
        assertEquals(listOf("192.0.2.3"), combined.all().single().hosts)
        assertNull(kv.get("pcremote_pcs"))
        vault.clearSession(session)
        combined.upsert(owned)
        assertTrue(combined.all().isEmpty())
        assertNull(kv.get("pcremote_pcs"))
    }

    @Test fun `independent manual access wins on duplicate host and survives account sign out`() {
        val kv = Memory()
        val vault = AccountStore(kv, Box) { 1000 }
        val combined = PcStore(kv, Box, OwnedPcAccess(vault))
        val session = session()
        vault.saveSession(session); vault.saveOwned(session, OwnedAccess(computer, pc))
        combined.upsert(pc.copy(token = "manual-token"))
        assertEquals("manual-token", combined.all().single().token)
        assertEquals("owned-token", combined.find(OwnedPcAccess.routeFor(computer))!!.token)
        vault.clearSession(session)
        assertEquals("manual-token", combined.all().single().token)
        assertNull(combined.find(OwnedPcAccess.routeFor(computer)))
    }

    @Test fun `late relay update cannot replace credentials from a new login`() {
        val kv = Memory()
        val vault = AccountStore(kv, Box) { 1000 }
        val access = OwnedPcAccess(vault)
        vault.saveSession(session()); vault.saveOwned(session(), OwnedAccess(computer, pc))
        val stale = access.all().single()
        val fresh = session("b".repeat(64))
        vault.saveSession(fresh); vault.saveOwned(fresh, OwnedAccess(computer, pc.copy(token = "new-token")))
        access.upsert(stale.copy(hosts = listOf("192.0.2.200")))
        assertEquals("new-token", access.all().single().token)
        assertEquals(pc.hosts, access.all().single().hosts)
    }

    @Test fun `account provenance never falls back into a standalone manual store`() {
        val kv = Memory()
        val manual = PcStore(kv, Box)
        manual.upsert(pc.copy(accountOwner = owner, accountComputer = computer))
        assertTrue(manual.all().isEmpty())
        assertNull(kv.get("pcremote_pcs"))
    }

    @Test fun `in memory sign out hides grants before slow preference erasure finishes`() {
        val vault = AccountStore(Memory(), Box) { 1000 }
        val session = session()
        vault.saveSession(session); vault.saveOwned(session, OwnedAccess(computer, pc))
        var visibleOwner: String? = owner
        val access = OwnedPcAccess(vault) { visibleOwner }
        assertEquals(1, access.all().size)
        visibleOwner = null
        assertNotNull(vault.session())
        assertTrue(access.all().isEmpty())
    }
}
