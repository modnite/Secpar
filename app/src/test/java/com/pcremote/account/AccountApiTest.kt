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
    private fun device(code: String = "ABCD2345", url: String = "https://accounts.secpar.win/activate?code=$code", secret: String = token) =
        buildJsonObject { put("device_code", secret); put("user_code", code); put("verification_uri_complete", url); put("expires_in", 600); put("interval", 5) }.toString()
    private fun computer(id: String = this.id, fingerprint: String = fp) =
        buildJsonObject { put("id", id); put("name", "Desk"); put("platform", "windows"); put("certificate_fp", fingerprint); put("linked", 1); put("online", 1) }
    private fun ready(link: String, fingerprint: String = fp) =
        buildJsonObject { put("id", id); put("link", link); put("certificate_fp", fingerprint); put("expires", 2000000000) }.toString()
    private fun link(fingerprint: String = fp, host: String = "192.0.2.1") = "secpar://pair?n=Desk&h=$host&p=47999&fp=$fingerprint&t=fixture-otp"
    private suspend fun rejects(kind: AccountError, block: suspend () -> Unit) {
        try { block(); fail("Expected a fixed account error") }
        catch (e: AccountException) { assertEquals(kind, e.kind); assertNull(e.cause) }
    }

    @Test fun `sign in pins approval URL and keeps credentials out of display strings`() = runBlocking {
        val f = Fake().apply { reply(device(), 201) }
        val result = AccountApi(transport = f).beginSignIn("Android")
        assertEquals(token, result.deviceCode)
        assertEquals(5L, result.interval)
        assertFalse(result.toString().contains(token))
        assertFalse(result.toString().contains(result.verificationUriComplete))
        assertEquals("https://accounts.secpar.win/auth/device", f.requests.single().url)
        assertNull(f.requests.single().bearer)
        assertFalse(f.requests.single().toString().contains("Android"))
    }

    @Test fun `approval rejects alternate origins paths queries fragments and malformed secrets`() = runBlocking {
        for (url in listOf("https://evil.example/activate?code=ABCD2345", "https://accounts.secpar.win/other?code=ABCD2345", "https://accounts.secpar.win/activate?code=ABCD2345&extra=1", "https://accounts.secpar.win/activate?code=ABCD2345#x")) {
            val f = Fake().apply { reply(device(url = url)) }
            rejects(AccountError.INVALID_RESPONSE) { AccountApi(transport = f).beginSignIn("Android") }
        }
        for (body in listOf(device(code = "abc12345"), device(secret = "z".repeat(64)), device(secret = "a".repeat(63)))) {
            val f = Fake().apply { reply(body) }
            rejects(AccountError.INVALID_RESPONSE) { AccountApi(transport = f).beginSignIn("Android") }
        }
    }

    @Test fun `poll handles pending slowdown and installs only validated bearer`() = runBlocking {
        val f = Fake().apply {
            reply("""{"error":"authorization_pending"}""", 400)
            reply("""{"error":"slow_down"}""", 429)
            reply("""{"access_token":"$token","token_type":"Bearer","expires_in":2592000}""")
            reply("""{"id":"$id","handle":"Player"}""")
        }
        val api = AccountApi(transport = f)
        assertSame(SignInPoll.Pending, api.pollSignIn(token))
        assertSame(SignInPoll.SlowDown, api.pollSignIn(token))
        val approved = api.pollSignIn(token) as SignInPoll.Authorized
        assertEquals(token, approved.token)
        assertFalse(approved.toString().contains(token))
        assertEquals(id, api.me().id)
        assertEquals(token, f.requests.last().bearer)
        assertTrue(f.requests.take(3).all { it.bearer == null })
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
        rejects(AccountError.INVALID_ARGUMENT) { api.pollSignIn("bad") }
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
}
