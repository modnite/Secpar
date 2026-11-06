package com.pcremote.account

import com.pcremote.agent.KeyValue
import com.pcremote.agent.PairedPc
import com.pcremote.agent.RelayInfo
import com.pcremote.agent.SecretBox
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

/** Browser credentials never enter the app. This is the revocable native-device session. */
class AccountSession(val user: AccountUser, val token: String, val expiresAt: Long) {
    override fun toString() = "AccountSession(redacted)"
}

class OwnedAccess(val computerId: String, val pc: PairedPc) {
    override fun toString() = "OwnedAccess(redacted)"
}

/**
 * Separate from manual QR pairings. Account access is encrypted in its entirety and may only
 * be committed by the still-current session, including after a delayed network response.
 * The Android preference adapter is shared by all callers; hold one store for each process.
 */
class AccountStore(private val kv: KeyValue, private val box: SecretBox, private val now: () -> Long = { System.currentTimeMillis() / 1000 }) {
    private val json = Json { ignoreUnknownKeys = true }

    @Synchronized
    fun session(): AccountSession? = readSession()?.takeIf { it.expiresAt > now() }

    @Synchronized
    fun saveSession(session: AccountSession) {
        require(uuid.matches(session.user.id) && token.matches(session.token) && session.expiresAt > now())
        require(session.user.handle.isNotBlank() && session.user.handle.length <= 80 && session.user.handle.none { it.isISOControl() })
        // Replacing a session must not expose grants issued to a previous native session.
        readSession()?.let { kv.put(ownedKey(it.user.id), null) }
        kv.put(ownedKey(session.user.id), null)
        kv.put(SESSION, box.seal(json.encodeToString(StoredSession.serializer(),
            StoredSession(session.user.id, session.user.handle, session.token, session.expiresAt))))
    }

    @Synchronized
    fun clearSession(expected: AccountSession): Boolean {
        if (!matches(readSession(), expected)) return false
        kv.put(ownedKey(expected.user.id), null)
        kv.put(SESSION, null)
        return true
    }

    @Synchronized
    fun owned(expected: AccountSession): List<OwnedAccess> {
        if (!matches(session(), expected)) return emptyList()
        return readGrants(expected).mapNotNull { grant -> runCatching { grant.access() }.getOrNull() }
    }

    @Synchronized
    fun saveOwned(expected: AccountSession, access: OwnedAccess): Boolean {
        if (!matches(session(), expected)) return false
        require(uuid.matches(access.computerId) && fingerprint.matches(access.pc.fingerprint))
        require(access.pc.id.isNotBlank() && access.pc.id.length <= 320 && access.pc.token.isNotBlank())
        require(access.pc.port in 1..65535 && access.pc.hosts.size <= 32)
        val grants = readGrants(expected).filterNot { it.computerId == access.computerId }
        require(grants.size < 256)
        val pc = access.pc
        val stored = StoredGrant(access.computerId, pc.id, pc.name, pc.hosts, pc.port, pc.fingerprint,
            pc.token, pc.macs, pc.publicHost, pc.relay?.url, pc.relay?.dev,
            pc.relay?.let { RelayInfo.b64url(it.ticket) })
        writeGrants(expected, grants + stored)
        return true
    }

    @Synchronized
    fun forgetOwned(expected: AccountSession, computerId: String): Boolean {
        if (!matches(session(), expected)) return false
        writeGrants(expected, readGrants(expected).filterNot { it.computerId == computerId })
        return true
    }

    private fun readSession(): AccountSession? = runCatching {
        val stored = json.decodeFromString(StoredSession.serializer(), box.open(kv.get(SESSION) ?: return null))
        if (!uuid.matches(stored.id) || !token.matches(stored.token) || stored.handle.isBlank() ||
            stored.handle.length > 80 || stored.handle.any { it.isISOControl() }) return null
        AccountSession(AccountUser(stored.id, stored.handle), stored.token, stored.expires)
    }.getOrNull()

    private fun readGrants(expected: AccountSession): List<StoredGrant> = runCatching {
        val text = kv.get(ownedKey(expected.user.id)) ?: return emptyList()
        val saved = json.decodeFromString(StoredGrants.serializer(), box.open(text))
        if (saved.owner != expected.user.id || saved.sessionBinding != binding(expected) || saved.grants.size > 256) emptyList()
        else saved.grants
    }.getOrDefault(emptyList())

    private fun writeGrants(expected: AccountSession, grants: List<StoredGrant>) {
        kv.put(ownedKey(expected.user.id), box.seal(json.encodeToString(StoredGrants.serializer(),
            StoredGrants(expected.user.id, binding(expected), grants))))
    }

    private fun binding(session: AccountSession) = java.security.MessageDigest.getInstance("SHA-256")
        .digest(session.token.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it) }

    private fun matches(current: AccountSession?, expected: AccountSession) =
        current != null && current.user.id == expected.user.id && current.token == expected.token

    private fun ownedKey(owner: String): String {
        require(uuid.matches(owner))
        return "secpar.account.owned.$owner"
    }

    private companion object {
        const val SESSION = "secpar.account.session"
        val uuid = Regex("[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}")
        val token = Regex("[a-f0-9]{64}")
        val fingerprint = Regex("[a-f0-9]{64}")
    }
}

@Serializable
private data class StoredSession(val id: String, val handle: String, val token: String, val expires: Long)
@Serializable
private data class StoredGrants(val owner: String, val sessionBinding: String, val grants: List<StoredGrant>)
@Serializable
private data class StoredGrant(
    val computerId: String, val id: String, val name: String, val hosts: List<String>, val port: Int,
    val fingerprint: String, val token: String, val macs: List<String>, val publicHost: String? = null,
    val relayUrl: String? = null, val relayDevice: String? = null, val relayTicket: String? = null,
) {
    fun access(): OwnedAccess {
        require(Regex("[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}").matches(computerId))
        require(Regex("[a-f0-9]{64}").matches(fingerprint) && port in 1..65535 && token.isNotBlank())
        val relay = if (relayUrl == null && relayDevice == null && relayTicket == null) null else {
            require(relayUrl != null && relayDevice != null && relayTicket != null)
            val ticket = RelayInfo.b64url(relayTicket)?.also { require(it.size == 32) } ?: error("Invalid saved access")
            RelayInfo(relayUrl, id, relayDevice, ticket)
        }
        return OwnedAccess(computerId, PairedPc(id, name, hosts, port, fingerprint, token, macs, publicHost, relay))
    }
}
