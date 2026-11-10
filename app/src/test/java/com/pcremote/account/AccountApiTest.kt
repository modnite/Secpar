package com.pcremote.account

import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test
import java.net.URLEncoder

class AccountApiTest {
    private val id = "00000000-0000-4000-8000-000000000001"
    private val token = "d".repeat(64)
    private val fp = "a".repeat(64)
    private class Fake : AccountTransport {
        val requests = mutableListOf<AccountRequest>()
        val replies = ArrayDeque<AccountResponse>()
        fun reply(body: String, status: Int = 200) { replies.add(AccountResponse(status, body.toByteArray())) }
        override fun execute(request: AccountRequest): AccountResponse { requests.add(request); return replies.removeFirst() }
    }
    private fun computer(id: String = this.id, fingerprint: String = fp) =
        buildJsonObject { put("id", id); put("name", "Desk"); put("platform", "windows"); put("certificate_fp", fingerprint); put("linked", 1); put("online", 1) }
    private fun ready(link: String, fingerprint: String = fp) =
        buildJsonObject { put("id", id); put("link", link); put("certificate_fp", fingerprint); put("expires", 2000000000) }.toString()
    private fun link(fingerprint: String = fp, host: String = "192.0.2.1") = "secpar://pair?n=Desk&h=$host&p=47999&fp=$fingerprint&t=fixture-otp"
    private suspend fun rejects(kind: AccountError, block: suspend () -> Unit) {
        try { block(); fail("Expected a fixed account error") }
        catch (e: AccountException) { assertEquals(kind, e.kind); assertNull(e.cause) }
    }

    @Test fun `password sign in sends the form and installs only a validated bearer`() = runBlocking {
        val f = Fake().apply {
            reply("""{"token":"$token","expires_at":${System.currentTimeMillis() / 1000 + 2592000},"user":{"id":"$id","handle":"Player"}}""")
            reply("""{"id":"$id","handle":"Player"}""")
        }
        val api = AccountApi(transport = f)
        val result = api.passwordSignIn("me@example.com", "correct horse")
        assertEquals(token, result.token)
        assertFalse(result.toString().contains(token))
        assertTrue(result.expiresIn in 1..2592000)
        assertEquals("https://accounts.secpar.win/v1/app/sign-in", f.requests[0].url)
        assertNull(f.requests[0].bearer)
        assertEquals(id, api.me().id)
        assertEquals(token, f.requests.last().bearer)
    }

    @Test fun `a wrong password shows the service's own plain words and installs nothing`() = runBlocking {
        val f = Fake().apply { reply("""{"error":"bad_credentials","message":"The email or password is wrong, or the email is not confirmed yet."}""", 401) }
        val api = AccountApi(transport = f)
        try { api.passwordSignIn("me@example.com", "nope nope nope"); fail() }
        catch (e: AccountException) { assertEquals("The email or password is wrong, or the email is not confirmed yet.", e.message) }
        rejects(AccountError.SIGN_IN_REQUIRED) { api.me() }
    }

    @Test fun `a sign in answer with a bad token or no expiry is refused`() = runBlocking {
        for (body in listOf("""{"token":"zz","expires_at":9999999999}""", """{"token":"$token"}""", """{"token":"$token","expires_at":"soon"}""")) {
            val f = Fake().apply { reply(body) }
            rejects(AccountError.INVALID_RESPONSE) { AccountApi(transport = f).passwordSignIn("me@example.com", "correct horse") }
        }
    }

    @Test fun `the other forms post to fixed routes with only known fields`() = runBlocking {
        val f = Fake().apply { reply("""{"sent":true}""", 202); reply("""{"done":true}""") }
        val api = AccountApi(transport = f)
        api.appAuth("register", mapOf("email" to "me@example.com", "password" to "correct horse", "handle" to "Mia"))
        api.appAuth("verify", mapOf("email" to "me@example.com", "code" to "123456"))
        assertEquals("https://accounts.secpar.win/v1/app/register", f.requests[0].url)
        assertEquals("https://accounts.secpar.win/v1/app/verify", f.requests[1].url)
        assertTrue(f.requests.all { it.bearer == null })
        rejects(AccountError.INVALID_ARGUMENT) { api.appAuth("sign-in", mapOf("email" to "a@b.co")) }
        rejects(AccountError.INVALID_ARGUMENT) { api.appAuth("../me", mapOf("email" to "a@b.co")) }
        rejects(AccountError.INVALID_ARGUMENT) { api.appAuth("verify", mapOf("admin" to "yes")) }
        assertEquals(2, f.requests.size)
    }

    @Test fun `the sign in form checks input the way the service does`() {
        assertTrue(SignInRules.emailOk("a@b.co") && !SignInRules.emailOk("a@b") && !SignInRules.emailOk("a b@c.de") && !SignInRules.emailOk("@b.co"))
        assertTrue(SignInRules.handleOk("Mia_1") && !SignInRules.handleOk("ab") && !SignInRules.handleOk("a-b") && !SignInRules.handleOk("a".repeat(31)))
        assertTrue(SignInRules.passwordOk("12345678") && !SignInRules.passwordOk("1234567"))
        assertEquals("Enter your password.", SignInRules.problem(SignInStep.SIGN_IN, false, "a@b.co", "", "", ""))
        assertEquals("Enter the six digit code from the email.", SignInRules.problem(SignInStep.CODE, false, "a@b.co", "", "", "12a456"))
        assertNull(SignInRules.problem(SignInStep.CODE, false, "a@b.co", "", "", "123 456"))
        assertNull(SignInRules.problem(SignInStep.CODE, true, "a@b.co", "long enough", "", "123456"))
    }

    @Test fun `invalid session and path identifiers never reach transport`() = runBlocking {
        val f = Fake()
        val api = AccountApi(token, f)
        for (bad in listOf("../me", "$id/extra", "", "a".repeat(36))) {
            rejects(AccountError.INVALID_ARGUMENT) { api.beginOwnerPair(bad, "Android") }
            rejects(AccountError.INVALID_ARGUMENT) { api.pollOwnerPair(bad) }
            rejects(AccountError.INVALID_ARGUMENT) { api.cancelOwnerPair(bad) }
        }
        rejects(AccountError.INVALID_ARGUMENT) { api.setSession("bad") }
        assertTrue(f.requests.isEmpty())
    }

    @Test fun `computer list validates identity fingerprint flags and count`() = runBlocking {
        val f = Fake().apply { reply(JsonArray(List(256) { computer() }).toString()) }
        assertEquals(256, AccountApi(token, f).computers().size)
        for (body in listOf(JsonArray(List(257) { computer() }).toString(), "[${computer(id = "../pc")}]", "[${computer(fingerprint = "bad")}]", """[{"id":"$id","name":"Desk","platform":"windows","certificate_fp":"$fp","linked":2,"online":1}]""")) {
            val bad = Fake().apply { reply(body) }
            rejects(AccountError.INVALID_RESPONSE) { AccountApi(token, bad).computers() }
        }
    }

    @Test fun `owner request ready and cancel use the account session`() = runBlocking {
        val f = Fake().apply {
            reply("""{"id":"$id","state":"pending","expires":2000000000}""", 201)
            reply("""{"id":"$id","state":"pending","expires":2000000000}""", 202)
            reply(ready(link()))
            reply("""{"cancelled":true}""")
        }
        val api = AccountApi(token, f)
        assertEquals(id, api.beginOwnerPair(id, "Android").id)
        assertTrue(api.pollOwnerPair(id) is OwnerPairPoll.Pending)
        val result = api.pollOwnerPair(id) as OwnerPairPoll.Ready
        assertEquals(fp, result.pairing.fingerprint)
        assertFalse(result.toString().contains("fixture-otp"))
        api.cancelOwnerPair(id)
        assertEquals(listOf("POST", "GET", "GET", "DELETE"), f.requests.map { it.method })
        assertTrue(f.requests.all { it.bearer == token && it.url.startsWith("https://accounts.secpar.win/v1/owner-pairings") })
    }

    @Test fun `ready rejects mismatched fp duplicate fields hidden paths and DNS hosts`() = runBlocking {
        for (link in listOf(link("b".repeat(64)), link() + "&fp=$fp", link(host = "example.com"), link().replace("pair?", "pair/hidden?"), link() + "#hidden", link().replace("secpar://", "pcremote://"))) {
            val f = Fake().apply { reply(ready(link)) }
            rejects(AccountError.INVALID_RESPONSE) { AccountApi(token, f).pollOwnerPair(id) }
        }
    }

    @Test fun `relay ready must bind its PC parameter`() = runBlocking {
        for (pc in listOf("pc-one", "pc-other")) {
            val relay = URLEncoder.encode("wss://relay.example/v1/phone?pc=$pc&sig=fixture", "UTF-8")
            val f = Fake().apply { reply(ready("secpar://pair?h=&p=47999&fp=$fp&t=fixture&pc=pc-one&r=$relay")) }
            val api = AccountApi(token, f)
            if (pc == "pc-one") assertEquals("pc-one", (api.pollOwnerPair(id) as OwnerPairPoll.Ready).pairing.pcId)
            else rejects(AccountError.INVALID_RESPONSE) { api.pollOwnerPair(id) }
        }
    }

    @Test fun `errors are sanitized and response bodies bounded`() = runBlocking {
        for (body in listOf("not-json-private-secret", " ".repeat(256 * 1024 + 1))) {
            val f = Fake().apply { reply(body) }
            rejects(AccountError.INVALID_RESPONSE) { AccountApi(token, f).me() }
        }
        val f = Fake().apply { reply("""{"error":"session_expired","private":"$token"}""", 401) }
        try { AccountApi(token, f).me(); fail("Expected failure") }
        catch (e: AccountException) { assertEquals(AccountError.SIGN_IN_REQUIRED, e.kind); assertFalse(e.toString().contains(token)) }
    }

    @Test fun `live logout contract accepts an empty 204 response`() = runBlocking {
        val f = Fake().apply { reply("", 204) }
        val api = AccountApi(token, f)
        api.logout()
        rejects(AccountError.SIGN_IN_REQUIRED) { api.me() }
        assertEquals("https://accounts.secpar.win/auth/logout", f.requests.single().url)
    }

    @Test fun `failed logout still clears bearer locally`() = runBlocking {
        val f = Fake().apply { reply("""{"error":"unavailable"}""", 503) }
        val api = AccountApi(token, f)
        rejects(AccountError.REQUEST_FAILED) { api.logout() }
        rejects(AccountError.SIGN_IN_REQUIRED) { api.me() }
        assertEquals(1, f.requests.size)
    }

    private val friendId = "00000000-0000-4000-8000-0000000000f1"

    @Test fun `friend list and block list parse and use the account session`() = runBlocking {
        val f = Fake().apply {
            reply("""[{"id":"$friendId","handle":"river","state":"accepted","requester":"$id"}]""")
            reply("""[{"id":"$friendId","handle":"noisy"}]""")
        }
        val api = AccountApi(token, f)
        assertEquals(Friend(friendId, "river", "accepted", id), api.friends().single())
        assertEquals(AccountUser(friendId, "noisy"), api.blocks().single())
        assertEquals(listOf("GET https://accounts.secpar.win/v1/friends", "GET https://accounts.secpar.win/v1/blocks"),
            f.requests.map { it.method + " " + it.url })
        assertTrue(f.requests.all { it.bearer == token })
    }

    @Test fun `friend actions use the desktop verbs and paths`() = runBlocking {
        val f = Fake().apply { repeat(5) { reply("", 204) } }
        val api = AccountApi(token, f)
        FriendAction.entries.forEach { api.friendAction(it, if (it == FriendAction.INVITE) "river_2" else friendId) }
        assertEquals(listOf(
            "PUT /v1/friends/by-username/river_2", "POST /v1/friends/$friendId/accept", "DELETE /v1/friends/$friendId",
            "PUT /v1/blocks/$friendId", "DELETE /v1/blocks/$friendId"),
            f.requests.map { it.method + " " + it.url.removePrefix("https://accounts.secpar.win") })
    }

    @Test fun `a username never reaches the URL path unless it is clean`() = runBlocking {
        val f = Fake()
        for (bad in listOf("../me", "river/accept", "river?x=1", "river#x", "%2e%2e", "Ri ver", "@river", "River", "ab", "a".repeat(31), "", "river\n", "r\u0131ver"))
            rejects(AccountError.BAD_NAME) { AccountApi(token, f).friendAction(FriendAction.INVITE, bad) }
        for (action in FriendAction.entries.filter { it != FriendAction.INVITE })
            rejects(AccountError.BAD_NAME) { AccountApi(token, f).friendAction(action, "river") }
        assertTrue(f.requests.isEmpty())
    }

    @Test fun `friend error codes map to the desktop messages`() = runBlocking {
        for ((code, kind, text) in listOf(
            Triple("account_unavailable", AccountError.FRIEND_UNAVAILABLE, "No one with that username can be added."),
            Triple("incoming_request_not_found", AccountError.REQUEST_GONE, "This friend request is no longer available."),
            Triple("session_expired", AccountError.SIGN_IN_REQUIRED, "Your session expired. Sign in again."))) {
            val f = Fake().apply { reply("""{"error":"$code"}""", 400) }
            try { AccountApi(token, f).friendAction(FriendAction.INVITE, "river"); fail("expected $code") }
            catch (e: AccountException) { assertEquals(kind, e.kind); assertEquals(text, e.message) }
        }
        val limited = Fake().apply { reply("""{"error":"rate_limited"}""", 429) }
        try { AccountApi(token, limited).friends(); fail() }
        catch (e: AccountException) { assertEquals("Too many attempts. Please wait and try again.", e.message) }
    }

    @Test fun `bodyless post and put get an empty body because OkHttp refuses them`() {
        for (m in listOf("POST", "PUT")) assertEquals(0L, requestBody(m, null)!!.contentLength())
        assertNull(requestBody("GET", null))
        assertNull(requestBody("DELETE", null))
        assertTrue(requestBody("POST", "{}")!!.contentLength() > 0)
        // the real builder: this threw "method PUT must have a request body" before
        okhttp3.Request.Builder().url("https://accounts.secpar.win/v1/friends/by-username/bob").method("PUT", requestBody("PUT", null)).build()
    }
}
