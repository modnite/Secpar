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
private val DIRECTORY_QUERY = Regex("q=[a-z0-9_]{1,30}")
private val UUID = Regex("[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}")
private const val UUID_PATTERN = "[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}"
private val PERMISSIONS = setOf("view", "audio", "gamepad", "keyboard", "mouse", "clipboard", "files")
private val REQUEST_STATE = Regex("[a-z_]{1,20}")
private val GRANT_TEXT = Regex("[A-Za-z0-9_.-]{1,4096}")
/** Every path this client calls. The account session is never sent anywhere else on the service. */
private val ALLOWED_PATHS = listOf(
    Regex("/auth/(device|device/token|logout)"),
    Regex("/v1/(me|me/visibility|computers|friends|blocks|directory|requests|friend-computers|owner-pairings)"),
    Regex("/v1/friends/by-username/[a-z0-9_]{3,30}"),
    Regex("/v1/friends/$UUID_PATTERN(/accept|/settings)?"),
    Regex("/v1/blocks/$UUID_PATTERN"),
    Regex("/v1/requests/$UUID_PATTERN(/approve|/claim|/revoke)?"),
    Regex("/v1/owner-pairings/$UUID_PATTERN"),
)

/** True for an https URL on the account service with an allowed path, no fragment, and a query only on the directory. */
internal fun transportAllows(url: String): Boolean {
    val u = try { URI(url) } catch (_: Exception) { return false }
    val path = u.rawPath ?: return false
    val queryOk = u.rawQuery == null || (path == "/v1/directory" && DIRECTORY_QUERY.matches(u.rawQuery))
    return u.scheme == "https" && u.rawAuthority == "accounts.secpar.win" && queryOk && u.rawFragment == null &&
        ALLOWED_PATHS.any { it.matches(path) }
}

enum class AccountError { NETWORK, INVALID_RESPONSE, INVALID_ARGUMENT, SIGN_IN_REQUIRED, EXPIRED, PENDING, SLOW_DOWN, UNAVAILABLE, RATE_LIMITED, REQUEST_FAILED,
    FRIEND_UNAVAILABLE, REQUEST_GONE, BAD_NAME, INPUT_NOT_ALLOWED, REQUEST_ANSWERED,
    HOST_NOT_SHARING, GRANT_NOT_AVAILABLE, ACCESS_ENDED }
class AccountException(val kind: AccountError) : Exception(when (kind) {
    AccountError.NETWORK -> "Could not reach Secpar. Check your connection and try again."
    AccountError.INVALID_RESPONSE -> "Unexpected account response."
    AccountError.INVALID_ARGUMENT -> "Invalid account request."
    AccountError.SIGN_IN_REQUIRED -> "Your session expired. Sign in again."
    AccountError.EXPIRED -> "Sign-in expired. Please start again."
    AccountError.PENDING -> "Waiting for browser approval."
    AccountError.SLOW_DOWN -> "Please wait before trying again."
    AccountError.RATE_LIMITED -> "Too many attempts. Please wait and try again."
    AccountError.FRIEND_UNAVAILABLE -> "No one with that username can be added."
    AccountError.REQUEST_GONE -> "This friend request is no longer available."
    AccountError.BAD_NAME -> "Enter a username of 3 to 30 letters, numbers or underscores."
    AccountError.INPUT_NOT_ALLOWED -> "This friend may not use the keyboard or mouse."
    AccountError.REQUEST_ANSWERED -> "This request was already answered or has expired."
    AccountError.HOST_NOT_SHARING -> "This computer is not letting friends connect right now."
    AccountError.GRANT_NOT_AVAILABLE -> "This request was already used. Ask again."
    AccountError.ACCESS_ENDED -> "Access to this computer ended or is not allowed."
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
        if (!transportAllows(request.url)) throw AccountException(AccountError.INVALID_ARGUMENT)
        val req = Request.Builder().url(request.url).header("Accept", "application/json")
            .apply { request.bearer?.let { header("Authorization", "Bearer $it") } }
            .method(request.method, requestBody(request.method, request.body)).build()
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

data class AccountUser(val id: String, val handle: String, val visible: Boolean = false)
/** [input] is "view" or "full" and [connect] is "ask" or "always": what this friend may do on the computers you host. */
data class Friend(val id: String, val handle: String, val state: String, val requester: String,
    val input: String = "view", val connect: String = "ask",
    /** Keyboard and mouse are set apart. An older service sends only [input], which then decides both. */
    val keyboard: Boolean = input == "full", val mouse: Boolean = input == "full") {
    val accepted get() = state == "accepted"
    val pending get() = state == "pending"
}
/** One registered user who chose to appear in search. [relation] is none, requested, incoming or friend. */
data class DirectoryEntry(val handle: String, val cursor: String, val relation: String)
/** A connection request waiting for this account's answer. */
data class ConnectionRequest(val id: String, val owner: String, val guestHandle: String, val computerName: String?,
    val requested: List<String>, val state: String)
enum class Answer { ALLOW_CONNECTION, DENY_CONNECTION, ACCEPT_FRIEND, DECLINE_FRIEND }
data class OwnedComputer(val id: String, val name: String, val platform: String, val certificateFp: String?, val linked: Boolean, val online: Boolean)
/** A computer of an accepted friend that is online and lets friends connect. */
data class FriendComputer(val id: String, val name: String, val platform: String, val owner: String, val ownerHandle: String,
    /** What the owner lets this account do there. An older service leaves these out. */
    val connect: String = "ask", val keyboard: Boolean = false, val mouse: Boolean = false)
/** A request to use a friend's computer. [expires] is a Unix second. [permissions] is what the Worker kept. */
data class AccessRequest(val id: String, val state: String, val expires: Long, val permissions: List<String>) {
    val approved get() = state == "approved"
    val pending get() = state == "pending"
}
/** The one-time answer to a claim. It works once and for 60 seconds. */
class GuestGrant(val grant: String, val expiresAt: Long, val pc: String, val relay: String, val fingerprint: String?) {
    override fun toString() = "GuestGrant(redacted)"
}
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
    suspend fun directory(query: String): List<DirectoryEntry> = emptyList()
    suspend fun setVisible(visible: Boolean) {}
    /** Null leaves that setting as it is. */
    suspend fun friendSetting(id: String, input: String?, connect: String?) {}
    suspend fun friendInput(id: String, keyboard: Boolean?, mouse: Boolean?) {}
    suspend fun requests(): List<ConnectionRequest> = emptyList()
    suspend fun answer(answer: Answer, id: String) {}
    suspend fun friendComputers(): List<FriendComputer> = emptyList()
    suspend fun requestAccess(computer: String, permissions: List<String>): AccessRequest = throw AccountException(AccountError.UNAVAILABLE)
    suspend fun accessStatus(id: String): AccessRequest = throw AccountException(AccountError.UNAVAILABLE)
    suspend fun claimAccess(id: String): GuestGrant = throw AccountException(AccountError.UNAVAILABLE)
    suspend fun cancelAccess(id: String) {}
}

/** The calls the Friends page makes. Paths and verbs match the desktop client. */
enum class FriendAction(val method: String, val collection: String, val suffix: String = "") {
    INVITE("PUT", "friends/by-username"), ACCEPT("POST", "friends", "/accept"), REMOVE("DELETE", "friends"),
    BLOCK("PUT", "blocks"), UNBLOCK("DELETE", "blocks");
    /** [id] is a username for INVITE and an account id for every other action. */
    fun path(id: String) = "/v1/$collection/$id$suffix"
}

class AccountApi(sessionToken: String? = null, private val transport: AccountTransport = OkHttpAccountTransport(),
    private val now: () -> Long = { System.currentTimeMillis() / 1000 }) : AccountService {
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
        val visible = (o["visible"] as? JsonPrimitive)?.takeIf { !it.isString }?.booleanOrNull ?: false
        return AccountUser(o.text("id").also { valid(UUID.matches(it)) }, o.text("handle").also { valid(safeText(it, 80)) }, visible)
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
            val input = o.optionalChoice("input", "view", "full")
            fun flag(key: String) = (o[key] as? JsonPrimitive)?.takeIf { !it.isString }?.booleanOrNull ?: (input == "full")
            Friend(o.text("id").also { valid(UUID.matches(it)) }, o.text("handle").also { valid(safeText(it, 80)) },
                state, o.text("requester").also { valid(UUID.matches(it)) },
                input, o.optionalChoice("connect", "ask", "always"), flag("keyboard"), flag("mouse"))
        }
    }

    override suspend fun directory(query: String): List<DirectoryEntry> {
        val name = query.trim().removePrefix("@").lowercase()
        if (name.length > 30 || !name.all { it in 'a'..'z' || it in '0'..'9' || it == '_' }) return emptyList()
        val rows = call("GET", if (name.isEmpty()) "/v1/directory" else "/v1/directory?q=$name") as? JsonArray ?: invalid()
        valid(rows.size <= 20)
        return rows.map { row ->
            val o = row.objectValue()
            DirectoryEntry(o.text("handle").also { valid(safeText(it, 80)) }, o.text("cursor").also { valid(safeText(it, 80)) },
                o.text("relation").also { valid(it in setOf("none", "requested", "incoming", "friend")) })
        }
    }

    override suspend fun setVisible(visible: Boolean) {
        call("PUT", "/v1/me/visibility", buildJsonObject { put("visible", visible) })
    }

    override suspend fun friendSetting(id: String, input: String?, connect: String?) {
        pathId(id)
        argument(input == null || input == "view" || input == "full")
        argument(connect == null || connect == "ask" || connect == "always")
        argument(input != null || connect != null)
        call("PUT", "/v1/friends/$id/settings", buildJsonObject {
            input?.let { put("input", it) }
            connect?.let { put("connect", it) }
        })
    }

    override suspend fun friendInput(id: String, keyboard: Boolean?, mouse: Boolean?) {
        pathId(id)
        argument(keyboard != null || mouse != null)
        call("PUT", "/v1/friends/$id/settings", buildJsonObject {
            keyboard?.let { put("keyboard", it) }
            mouse?.let { put("mouse", it) }
        })
    }

    override suspend fun requests(): List<ConnectionRequest> {
        val rows = call("GET", "/v1/requests") as? JsonArray ?: invalid()
        valid(rows.size <= 1024)
        return rows.map { row ->
            val o = row.objectValue()
            val requested = Json.parseToJsonElement(o.text("requested")) as? JsonArray ?: invalid()
            val name = (o["computer_name"] as? JsonPrimitive)?.takeIf { it.isString }?.content
            ConnectionRequest(o.text("id").also { valid(UUID.matches(it)) }, o.text("owner").also { valid(UUID.matches(it)) },
                o.text("guest_handle").also { valid(safeText(it, 80)) }, name?.takeIf { safeText(it, 320) },
                requested.map { (it as? JsonPrimitive)?.takeIf { p -> p.isString }?.content ?: invalid() }, o.text("state"))
        }
    }

    override suspend fun answer(answer: Answer, id: String) {
        pathId(id)
        when (answer) {
            Answer.ACCEPT_FRIEND -> call("POST", "/v1/friends/$id/accept", obj())
            Answer.DECLINE_FRIEND -> call("DELETE", "/v1/friends/$id")
            Answer.DENY_CONNECTION -> call("POST", "/v1/requests/$id/revoke", obj())
            Answer.ALLOW_CONNECTION -> {
                val row = call("GET", "/v1/requests/$id").objectValue()
                val requested = Json.parseToJsonElement(row.text("requested")) as? JsonArray ?: invalid()
                call("POST", "/v1/requests/$id/approve", buildJsonObject { put("permissions", requested) })
            }
        }
    }

    override suspend fun friendComputers(): List<FriendComputer> {
        val rows = call("GET", "/v1/friend-computers") as? JsonArray ?: invalid()
        valid(rows.size <= 256)
        return rows.map { row ->
            val o = row.objectValue()
            FriendComputer(o.text("id").also { valid(UUID.matches(it)) }, o.text("name").also { valid(safeText(it, 320)) },
                o.text("platform").also { valid(safeText(it, 16)) }, o.text("owner").also { valid(UUID.matches(it)) },
                o.text("owner_handle").also { valid(safeText(it, 80)) },
                o.optionalChoice("connect", "ask", "always"),
                (o["keyboard"] as? JsonPrimitive)?.takeIf { !it.isString }?.booleanOrNull ?: false,
                (o["mouse"] as? JsonPrimitive)?.takeIf { !it.isString }?.booleanOrNull ?: false)
        }
    }

    override suspend fun requestAccess(computer: String, permissions: List<String>): AccessRequest {
        pathId(computer)
        argument(permissions.isNotEmpty() && permissions.all { it in PERMISSIONS } && permissions.toSet().size == permissions.size)
        val body = buildJsonObject {
            put("computer", computer)
            put("permissions", JsonArray(permissions.map { JsonPrimitive(it) }))
        }
        val o = call("POST", "/v1/requests", body).objectValue()
        val state = o.text("state").also { valid(it == "pending" || it == "approved") }
        // The Worker answers with expires_in (seconds from now). The docs say expires (a Unix second). Both work.
        val expires = o.optionalPositive("expires") ?: (now() + (o.optionalPositive("expires_in") ?: 300))
        val kept = (o["permissions"] as? JsonArray)?.mapNotNull { (it as? JsonPrimitive)?.takeIf { p -> p.isString }?.content?.takeIf { p -> p in PERMISSIONS } }
        return AccessRequest(o.text("id").also { valid(UUID.matches(it)) }, state, minOf(expires, now() + 900), kept ?: permissions)
    }

    override suspend fun accessStatus(id: String): AccessRequest {
        pathId(id)
        val o = call("GET", "/v1/requests/$id").objectValue()
        valid(o.text("id") == id)
        val state = o.text("state").also { valid(REQUEST_STATE.matches(it)) }
        val list = ((o["approved"] as? JsonPrimitive)?.takeIf { it.isString }?.content ?: (o["requested"] as? JsonPrimitive)?.takeIf { it.isString }?.content)
            ?.let { runCatching { Json.parseToJsonElement(it) as? JsonArray }.getOrNull() }
            ?.mapNotNull { (it as? JsonPrimitive)?.takeIf { p -> p.isString }?.content?.takeIf { p -> p in PERMISSIONS } }.orEmpty()
        return AccessRequest(id, state, minOf(o.positive("expires"), now() + 900), list)
    }

    override suspend fun claimAccess(id: String): GuestGrant {
        pathId(id)
        val o = call("POST", "/v1/requests/$id/claim", obj()).objectValue()
        val host = o["host"] as? JsonObject ?: invalid()
        val pc = host.text("pc"); val relay = host.text("relay")
        valid(com.pcremote.guest.GuestRelay.socketUrl(relay, pc) != null)
        val fp = (host["fingerprint"] as? JsonPrimitive)?.takeIf { it.isString }?.content
        valid(fp == null || HEX64.matches(fp))
        return GuestGrant(o.text("grant").also { valid(GRANT_TEXT.matches(it)) }, o.positive("expires_at"), pc, relay, fp)
    }

    override suspend fun cancelAccess(id: String) {
        pathId(id)
        call("POST", "/v1/requests/$id/revoke", obj())
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
        // The value goes into the URL path, so only a clean username or an account id gets through.
        val clean = if (action == FriendAction.INVITE) AccountText.cleanUsername(id) == id else UUID.matches(id)
        if (!clean) throw AccountException(AccountError.BAD_NAME)
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
                    code == "input_not_allowed" -> AccountError.INPUT_NOT_ALLOWED
                    code == "host_not_sharing" -> AccountError.HOST_NOT_SHARING
                    code == "grant_not_available" -> AccountError.GRANT_NOT_AVAILABLE
                    code == "access_expired_or_revoked" || code == "friend_access_required" -> AccountError.ACCESS_ENDED
                    code == "request_not_pending" || code == "request_not_found" -> AccountError.REQUEST_ANSWERED
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

/** OkHttp refuses a POST or PUT without a body, so friend invites, accepts and blocks send an empty one. */
internal fun requestBody(method: String, body: String?): okhttp3.RequestBody? =
    if (body == null && method in setOf("POST", "PUT", "PATCH")) "".toRequestBody(null)
    else body?.toRequestBody("application/json".toMediaType())

private fun invalid(): Nothing = throw AccountException(AccountError.INVALID_RESPONSE)
private fun valid(value: Boolean) { if (!value) invalid() }
private fun argument(value: Boolean) { if (!value) throw AccountException(AccountError.INVALID_ARGUMENT) }
private fun pathId(id: String) = argument(UUID.matches(id))
private fun safeText(value: String, max: Int) = value.isNotBlank() && value.length <= max && value.none { it < ' ' || it == '\u007f' }
private fun inputName(value: String, max: Int) = argument(safeText(value, max))
private fun JsonElement.objectValue() = this as? JsonObject ?: invalid()
private fun JsonObject.text(key: String) = (get(key) as? JsonPrimitive)?.takeIf { it.isString }?.content ?: invalid()
private fun JsonObject.positive(key: String) = (get(key) as? JsonPrimitive)?.takeIf { !it.isString }?.longOrNull?.takeIf { it > 0 } ?: invalid()
private fun JsonObject.optionalPositive(key: String) = (get(key) as? JsonPrimitive)?.takeIf { !it.isString }?.longOrNull?.takeIf { it > 0 }
private fun JsonObject.optionalChoice(key: String, default: String, other: String): String {
    val value = (get(key) as? JsonPrimitive)?.takeIf { it.isString }?.content ?: return default
    valid(value == default || value == other)
    return value
}
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
