package com.pcremote.account

import com.pcremote.agent.PairUri
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.*
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import java.net.URI
import java.net.URLDecoder
import java.util.concurrent.TimeUnit

/** Only this fixed service receives the account session. No redirects or custom trust manager. */
private const val ORIGIN = "https://accounts.secpar.win"
private const val MAX_BODY = 256 * 1024
private val HEX64 = Regex("[a-f0-9]{64}")
private val UUID = Regex("[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}")

enum class AccountError { NETWORK, INVALID_RESPONSE, INVALID_ARGUMENT, SIGN_IN_REQUIRED, EXPIRED, PENDING, SLOW_DOWN, UNAVAILABLE, RATE_LIMITED, REQUEST_FAILED,
    FRIEND_UNAVAILABLE, OWN_CODE, REQUEST_GONE, BAD_CODE }
class AccountException(val kind: AccountError) : Exception(when (kind) {
    AccountError.NETWORK -> "Could not reach Secpar. Check your connection and try again."
    AccountError.INVALID_RESPONSE -> "Unexpected account response."
    AccountError.INVALID_ARGUMENT -> "Invalid account request."
    AccountError.SIGN_IN_REQUIRED -> "Your session expired. Sign in again."
    AccountError.EXPIRED -> "Sign-in expired. Please start again."
    AccountError.PENDING -> "Waiting for browser approval."
    AccountError.SLOW_DOWN -> "Please wait before trying again."
    AccountError.RATE_LIMITED -> "Too many attempts. Please wait and try again."
    AccountError.FRIEND_UNAVAILABLE -> "Friend code was not found or is unavailable."
    AccountError.OWN_CODE -> "That is your own friend code."
    AccountError.REQUEST_GONE -> "This friend request is no longer available."
    AccountError.BAD_CODE -> "Paste your friend's complete friend code."
    AccountError.UNAVAILABLE -> "Computer pairing is no longer available."
    AccountError.REQUEST_FAILED -> "The account request failed. Please try again."
})

/** Transport seam for JVM tests. Request/response string representations never include payloads. */
class AccountRequest(val method: String, val url: String, val bearer: String?, val body: String?) {
    override fun toString() = "AccountRequest($method, redacted)"
}
class AccountResponse(val status: Int, val body: ByteArray) {
    override fun toString() = "AccountResponse($status, redacted)"
}
fun interface AccountTransport { fun execute(request: AccountRequest): AccountResponse }

class OkHttpAccountTransport : AccountTransport {
    private val client = OkHttpClient.Builder().followRedirects(false).followSslRedirects(false)
        .connectTimeout(8, TimeUnit.SECONDS).readTimeout(12, TimeUnit.SECONDS)
        .writeTimeout(12, TimeUnit.SECONDS).callTimeout(20, TimeUnit.SECONDS).build()
    override fun execute(request: AccountRequest): AccountResponse {
        val u = URI(request.url)
        if (u.scheme != "https" || u.rawAuthority != "accounts.secpar.win" || u.rawQuery != null ||
            u.rawFragment != null || !u.rawPath.startsWith("/")) throw AccountException(AccountError.INVALID_ARGUMENT)
        val req = Request.Builder().url(request.url).header("Accept", "application/json")
            .apply { request.bearer?.let { header("Authorization", "Bearer $it") } }
            .method(request.method, request.body?.toRequestBody("application/json".toMediaType())).build()
        return client.newCall(req).execute().use { response ->
            val body = response.body ?: throw AccountException(AccountError.INVALID_RESPONSE)
            if (body.contentLength() > MAX_BODY) throw AccountException(AccountError.INVALID_RESPONSE)
            val output = java.io.ByteArrayOutputStream()
            val source = body.byteStream()
            val buffer = ByteArray(8192)
            while (true) {
                val count = source.read(buffer, 0, minOf(buffer.size, MAX_BODY + 1 - output.size()))
                if (count < 0) break
                output.write(buffer, 0, count)
                if (output.size() > MAX_BODY) throw AccountException(AccountError.INVALID_RESPONSE)
            }
            val bytes = output.toByteArray()
            if (bytes.size > MAX_BODY) throw AccountException(AccountError.INVALID_RESPONSE)
            AccountResponse(response.code, bytes)
        }
    }
}

data class AccountUser(val id: String, val handle: String)
data class Friend(val id: String, val handle: String, val state: String, val requester: String) {
    val accepted get() = state == "accepted"
    val pending get() = state == "pending"
}
data class OwnedComputer(val id: String, val name: String, val platform: String, val certificateFp: String?, val linked: Boolean, val online: Boolean)
class DeviceSignIn(val deviceCode: String, val userCode: String, val verificationUriComplete: String, val expiresIn: Long, val interval: Long) {
    override fun toString() = "DeviceSignIn(redacted)"
}
sealed interface SignInPoll {
    data object Pending : SignInPoll
    data object SlowDown : SignInPoll
    class Authorized(val token: String, val expiresIn: Long) : SignInPoll {
        override fun toString() = "Authorized(redacted)"
    }
}
data class OwnerPairPending(val id: String, val expires: Long)
sealed interface OwnerPairPoll {
    data class Pending(val id: String, val expires: Long) : OwnerPairPoll
    class Ready(val pairing: PairUri, val certificateFp: String, val expires: Long) : OwnerPairPoll {
        override fun toString() = "OwnerPairReady(redacted)"
    }
}

/** One instance belongs to one login generation; never reuse it across sign-in/sign-out. */
interface AccountService {
    suspend fun beginSignIn(name: String): DeviceSignIn
    suspend fun pollSignIn(deviceCode: String): SignInPoll
    suspend fun me(): AccountUser
    suspend fun computers(): List<OwnedComputer>
    suspend fun beginOwnerPair(computerId: String, deviceName: String): OwnerPairPending
    suspend fun pollOwnerPair(requestId: String): OwnerPairPoll
    suspend fun cancelOwnerPair(requestId: String)
    suspend fun logout()
    suspend fun friends(): List<Friend> = emptyList()
    suspend fun blocks(): List<AccountUser> = emptyList()
    /** One change to the friend or block lists. */
    suspend fun friendAction(action: FriendAction, id: String) {}
}

/** The calls the Friends page makes. Paths and verbs match the desktop client. */
enum class FriendAction(val method: String, val collection: String, val suffix: String = "") {
    INVITE("PUT", "friends"), ACCEPT("POST", "friends", "/accept"), REMOVE("DELETE", "friends"),
    BLOCK("PUT", "blocks"), UNBLOCK("DELETE", "blocks");
    fun path(id: String) = "/v1/$collection/$id$suffix"
}

class AccountApi(sessionToken: String? = null, private val transport: AccountTransport = OkHttpAccountTransport()) : AccountService {
    @Volatile private var bearer: String? = null
    init { setSession(sessionToken) }
    fun setSession(token: String?) {
        if (token != null && !HEX64.matches(token)) throw AccountException(AccountError.INVALID_ARGUMENT)
        bearer = token
    }

    override suspend fun beginSignIn(name: String): DeviceSignIn {
        inputName(name, 60)
        val o = call("POST", "/auth/device", obj("name" to name), authenticated = false).objectValue()
        val code = o.text("user_code")
        val uri = o.text("verification_uri_complete")
        valid(Regex("[A-Z0-9]{8}").matches(code) && uri == "$ORIGIN/activate?code=$code")
        val secret = o.text("device_code").also { valid(HEX64.matches(it)) }
        val expiry = o.positive("expires_in").also { valid(it <= 600) }
        val interval = o.positive("interval").also { valid(it <= 60) }
        return DeviceSignIn(secret, code, uri, expiry, interval)
    }

    override suspend fun pollSignIn(deviceCode: String): SignInPoll {
        argument(HEX64.matches(deviceCode))
        val o = try { call("POST", "/auth/device/token", obj("device_code" to deviceCode), authenticated = false).objectValue() }
        catch (e: AccountException) { return when (e.kind) {
            AccountError.PENDING -> SignInPoll.Pending
            AccountError.SLOW_DOWN -> SignInPoll.SlowDown
            else -> throw e
        } }
        val token = o.text("access_token").also { valid(HEX64.matches(it)) }
        valid(o.text("token_type") == "Bearer")
        val expires = o.positive("expires_in")
        setSession(token)
        return SignInPoll.Authorized(token, expires)
    }

    override suspend fun me(): AccountUser {
        val o = call("GET", "/v1/me").objectValue()
        return AccountUser(o.text("id").also { valid(UUID.matches(it)) }, o.text("handle").also { valid(safeText(it, 80)) })
    }

    override suspend fun computers(): List<OwnedComputer> {
        val rows = call("GET", "/v1/computers") as? JsonArray ?: invalid()
        valid(rows.size <= 256)
        return rows.map { row ->
            val o = row.objectValue()
            val fp = o["certificate_fp"].let { if (it == JsonNull) null else (it as? JsonPrimitive)?.takeIf { p -> p.isString }?.content ?: invalid() }
            valid(fp == null || HEX64.matches(fp))
            val linked = o.bit("linked")
            valid(!linked || fp != null)
            OwnedComputer(o.text("id").also { valid(UUID.matches(it)) },
                o.text("name").also { valid(safeText(it, 320)) },
                o.text("platform").also { valid(safeText(it, 16)) }, fp, linked, o.bit("online"))
        }
    }

    override suspend fun friends(): List<Friend> {
        val rows = call("GET", "/v1/friends") as? JsonArray ?: invalid()
        valid(rows.size <= 1024)
        return rows.map { row ->
            val o = row.objectValue()
            val state = o.text("state").also { valid(it == "accepted" || it == "pending") }
            Friend(o.text("id").also { valid(UUID.matches(it)) }, o.text("handle").also { valid(safeText(it, 80)) },
                state, o.text("requester").also { valid(UUID.matches(it)) })
        }
    }

    override suspend fun blocks(): List<AccountUser> {
        val rows = call("GET", "/v1/blocks") as? JsonArray ?: invalid()
        valid(rows.size <= 1024)
        return rows.map { row ->
            val o = row.objectValue()
            AccountUser(o.text("id").also { valid(UUID.matches(it)) }, o.text("handle").also { valid(safeText(it, 80)) })
        }
    }

    override suspend fun friendAction(action: FriendAction, id: String) {
        if (!UUID.matches(id)) throw AccountException(AccountError.BAD_CODE)
        call(action.method, action.path(id))
    }

    override suspend fun beginOwnerPair(computerId: String, deviceName: String): OwnerPairPending {
        pathId(computerId); inputName(deviceName, 80)
        val o = call("POST", "/v1/owner-pairings", obj("computer" to computerId, "device" to deviceName)).objectValue()
        valid(o.text("state") == "pending")
        return OwnerPairPending(o.text("id").also { valid(UUID.matches(it)) }, o.positive("expires"))
    }

    override suspend fun pollOwnerPair(requestId: String): OwnerPairPoll {
        pathId(requestId)
        val o = call("GET", "/v1/owner-pairings/$requestId").objectValue()
        valid(o.text("id") == requestId)
        val expires = o.positive("expires")
        if ((o["state"] as? JsonPrimitive)?.content == "pending") return OwnerPairPoll.Pending(requestId, expires)
        val fp = o.text("certificate_fp").also { valid(HEX64.matches(it)) }
        val pair = checkedPair(o.text("link"), fp)
        return OwnerPairPoll.Ready(pair, fp, expires)
    }

    override suspend fun cancelOwnerPair(requestId: String) {
        pathId(requestId)
        valid(call("DELETE", "/v1/owner-pairings/$requestId").objectValue()["cancelled"] == JsonPrimitive(true))
    }

    /** Clears the in-memory bearer even if server revocation cannot be reached. */
    override suspend fun logout() {
        try { if (bearer != null) call("POST", "/auth/logout", obj()) }
        finally { bearer = null }
    }

    private suspend fun call(method: String, path: String, body: JsonObject? = null, authenticated: Boolean = true): JsonElement = withContext(Dispatchers.IO) {
        val token = if (authenticated) bearer ?: throw AccountException(AccountError.SIGN_IN_REQUIRED) else null
        try {
            val response = transport.execute(AccountRequest(method, ORIGIN + path, token, body?.toString()))
            valid(response.body.size <= MAX_BODY)
            val data = if (response.body.isEmpty()) JsonNull else Json.parseToJsonElement(response.body.decodeToString(throwOnInvalidSequence = true))
            if (response.status !in 200..299) {
                val code = (data as? JsonObject)?.get("error")?.let { it as? JsonPrimitive }?.content
                throw AccountException(when {
                    code == "authorization_pending" -> AccountError.PENDING
                    code == "slow_down" -> AccountError.SLOW_DOWN
                    response.status == 401 || code == "session_expired" || code == "sign_in_required" -> AccountError.SIGN_IN_REQUIRED
                    code == "account_unavailable" -> AccountError.FRIEND_UNAVAILABLE
                    code == "cannot_friend_yourself" -> AccountError.OWN_CODE
                    code == "incoming_request_not_found" -> AccountError.REQUEST_GONE
                    code == "device_code_expired_or_used" -> AccountError.EXPIRED
                    code == "pairing_request_unavailable" || code == "pairing_result_unavailable" || code == "computer_not_found" -> AccountError.UNAVAILABLE
                    response.status == 429 -> AccountError.RATE_LIMITED
                    else -> AccountError.REQUEST_FAILED
                })
            }
            data
        } catch (e: CancellationException) { throw e }
        catch (e: AccountException) { throw e }
        catch (_: java.io.IOException) { throw AccountException(AccountError.NETWORK) }
        catch (_: Exception) { throw AccountException(AccountError.INVALID_RESPONSE) }
    }
}

private fun invalid(): Nothing = throw AccountException(AccountError.INVALID_RESPONSE)
private fun valid(value: Boolean) { if (!value) invalid() }
private fun argument(value: Boolean) { if (!value) throw AccountException(AccountError.INVALID_ARGUMENT) }
private fun pathId(id: String) = argument(UUID.matches(id))
private fun safeText(value: String, max: Int) = value.isNotBlank() && value.length <= max && value.none { it < ' ' || it == '\u007f' }
private fun inputName(value: String, max: Int) = argument(safeText(value, max))
private fun JsonElement.objectValue() = this as? JsonObject ?: invalid()
private fun JsonObject.text(key: String) = (get(key) as? JsonPrimitive)?.takeIf { it.isString }?.content ?: invalid()
private fun JsonObject.positive(key: String) = (get(key) as? JsonPrimitive)?.takeIf { !it.isString }?.longOrNull?.takeIf { it > 0 } ?: invalid()
private fun JsonObject.bit(key: String): Boolean {
    val value = (get(key) as? JsonPrimitive)?.takeIf { !it.isString }?.intOrNull ?: invalid()
    valid(value == 0 || value == 1)
    return value == 1
}
private fun obj(vararg values: Pair<String, String>) = buildJsonObject { values.forEach { put(it.first, it.second) } }

/** Strict account bootstrap links: no duplicate fields, DNS names or hidden URL components. */
private fun checkedPair(link: String, fp: String): PairUri {
    valid(link.length <= 8192)
    return try {
        val uri = URI(link)
        valid(uri.scheme == "secpar" && uri.rawAuthority == "pair" && uri.rawPath.isNullOrEmpty() && uri.rawFragment == null)
        val params = query(uri.rawQuery ?: invalid())
        valid(params.keys.all { it in setOf("n", "h", "p", "fp", "t", "r", "pc") })
        valid(params["fp"] == fp)
        val pair = PairUri.parse(link) ?: invalid()
        valid(pair.fingerprint == fp && pair.otp.length <= 256 && pair.otp.none { it < ' ' || it == '\u007f' })
        valid(pair.hosts.size <= 16 && pair.hosts.all(::numericIp))
        pair.relayUrl?.let { relay ->
            val u = URI(relay)
            valid(u.scheme == "wss" && !u.host.isNullOrEmpty() && u.userInfo == null && u.fragment == null && u.path == "/v1/phone")
            valid(!pair.pcId.isNullOrBlank() && query(u.rawQuery ?: invalid())["pc"] == pair.pcId)
        }
        pair
    } catch (e: AccountException) { throw e }
    catch (_: Exception) { invalid() }
}
private fun query(raw: String): Map<String, String> {
    val result = linkedMapOf<String, String>()
    raw.split('&').forEach { field ->
        val i = field.indexOf('='); valid(i > 0)
        val key = URLDecoder.decode(field.substring(0, i), "UTF-8")
        val value = URLDecoder.decode(field.substring(i + 1), "UTF-8")
        valid(!result.containsKey(key)); result[key] = value
    }
    return result
}
private fun numericIp(value: String): Boolean {
    if (':' in value) return value.matches(Regex("[0-9a-fA-F:.]+")) && runCatching { java.net.InetAddress.getByName(value) is java.net.Inet6Address }.getOrDefault(false)
    val parts = value.split('.')
    return parts.size == 4 && parts.all { it.isNotEmpty() && it.length <= 3 && it.all { ch -> ch in '0'..'9' } && (it.toIntOrNull() ?: 256) <= 255 }
}
