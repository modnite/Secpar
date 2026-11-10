package com.pcremote.account

import com.pcremote.agent.KeyValue
import com.pcremote.agent.PairUri
import com.pcremote.agent.PairedPc
import com.pcremote.agent.SecretBox
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.TestScope
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.util.Base64

class FriendsConnectTest {
    private val me = "00000000-0000-4000-8000-000000000001"
    private val owner = "00000000-0000-4000-8000-000000000002"
    private val computer = "00000000-0000-4000-8000-000000000003"
    private val request = "00000000-0000-4000-8000-000000000004"
    private val token = "d".repeat(64)

    private class Fake : AccountTransport {
        val requests = mutableListOf<AccountRequest>()
        val replies = ArrayDeque<AccountResponse>()
        fun reply(body: String, status: Int = 200) { replies.add(AccountResponse(status, body.toByteArray())) }
        override fun execute(request: AccountRequest): AccountResponse { requests.add(request); return replies.removeFirst() }
    }
    private fun api(f: Fake) = AccountApi(token, f) { 1000L }
    private fun kind(block: () -> Unit): AccountError {
        try { block() } catch (e: AccountException) { return e.kind }
        fail("no exception"); throw IllegalStateException()
    }
    private val claimBody = """{"grant":"aaa.bbb.ccc","expires_at":1060,"host":{"pc":"pc_1-A","relay":"https://rendezvous.example.com","fingerprint":"${"c".repeat(64)}"}}"""

    // ---- the four routes -------------------------------------------------------------------------------

    @Test fun `friend computers are listed from the fixed path`() = runBlocking {
        val f = Fake().apply { reply("""[{"id":"$computer","name":"Studio PC","platform":"windows","owner":"$owner","owner_handle":"bob_1"}]""") }
        val rows = api(f).friendComputers()
        assertEquals(listOf(FriendComputer(computer, "Studio PC", "windows", owner, "bob_1")), rows)
        assertEquals("GET", f.requests.single().method)
        assertEquals("https://accounts.secpar.win/v1/friend-computers", f.requests.single().url)
        assertNull(f.requests.single().body)
    }

    @Test fun `a friend computer row with a bad id or a control character is refused`() = runBlocking {
        for (row in listOf("""{"id":"x","name":"a","platform":"linux","owner":"$owner","owner_handle":"b"}""",
            """{"id":"$computer","name":"a\u0007b","platform":"linux","owner":"$owner","owner_handle":"b"}""",
            """{"id":"$computer","name":"a","platform":"linux","owner_handle":"b"}""")) {
            val f = Fake().apply { reply("[$row]") }
            assertEquals(row, AccountError.INVALID_RESPONSE, kind { runBlocking { api(f).friendComputers() } })
        }
    }

    @Test fun `a request posts the computer and the permissions as fixed json`() = runBlocking {
        val f = Fake().apply { reply("""{"id":"$request","state":"pending","expires_in":300,"permissions":["view","audio"]}""", 201) }
        val answer = api(f).requestAccess(computer, listOf("view", "audio", "keyboard"))
        assertEquals(AccessRequest(request, "pending", 1300, listOf("view", "audio")), answer)
        val sent = f.requests.single()
        assertEquals("POST", sent.method)
        assertEquals("https://accounts.secpar.win/v1/requests", sent.url)
        val body = Json.parseToJsonElement(sent.body!!).jsonObject
        assertEquals(setOf("computer", "permissions"), body.keys)
        assertEquals(computer, body["computer"]!!.jsonPrimitive.content)
        assertEquals(listOf("view", "audio", "keyboard"), body["permissions"]!!.jsonArray.map { it.jsonPrimitive.content })
    }

    @Test fun `an answer with a Unix second expires is used as it is`() = runBlocking {
        val f = Fake().apply { reply("""{"id":"$request","state":"approved","expires":1234}""", 201) }
        val answer = api(f).requestAccess(computer, listOf("view"))
        assertEquals(1234L, answer.expires)
        assertTrue(answer.approved)
        assertEquals(listOf("view"), answer.permissions)
    }

    @Test fun `permissions that the service does not know never leave the phone`() {
        val f = Fake()
        for (bad in listOf(emptyList(), listOf("view", "root"), listOf("view", "view"), listOf("../x"))) {
            assertEquals(bad.toString(), AccountError.INVALID_ARGUMENT, kind { runBlocking { api(f).requestAccess(computer, bad) } })
        }
        assertEquals(AccountError.INVALID_ARGUMENT, kind { runBlocking { api(f).requestAccess("../x", listOf("view")) } })
        assertTrue(f.requests.isEmpty())
    }

    @Test fun `a view only friend asking for input only gets input_not_allowed`() {
        val f = Fake().apply { reply("""{"error":"input_not_allowed"}""", 403) }
        assertEquals(AccountError.INPUT_NOT_ALLOWED, kind { runBlocking { api(f).requestAccess(computer, listOf("keyboard", "mouse")) } })
    }

    @Test fun `polling reads the state and the expiry of one request`() = runBlocking {
        val f = Fake().apply { reply("""{"id":"$request","state":"approved","expires":1250,"requested":"[\"view\",\"audio\"]","approved":"[\"view\"]","active":false}""") }
        val answer = api(f).accessStatus(request)
        assertEquals(AccessRequest(request, "approved", 1250, listOf("view")), answer)
        assertEquals("GET", f.requests.single().method)
        assertEquals("https://accounts.secpar.win/v1/requests/$request", f.requests.single().url)
    }

    @Test fun `polling refuses another request's answer and a state with odd characters`() {
        val other = Fake().apply { reply("""{"id":"00000000-0000-4000-8000-000000000009","state":"pending","expires":1250}""") }
        assertEquals(AccountError.INVALID_RESPONSE, kind { runBlocking { api(other).accessStatus(request) } })
        val odd = Fake().apply { reply("""{"id":"$request","state":"Pending!","expires":1250}""") }
        assertEquals(AccountError.INVALID_RESPONSE, kind { runBlocking { api(odd).accessStatus(request) } })
    }

    @Test fun `a claim returns the grant and the host`() = runBlocking {
        val f = Fake().apply { reply(claimBody) }
        val grant = api(f).claimAccess(request)
        assertEquals("aaa.bbb.ccc", grant.grant)
        assertEquals(1060L, grant.expiresAt)
        assertEquals("pc_1-A", grant.pc)
        assertEquals("https://rendezvous.example.com", grant.relay)
        assertEquals("c".repeat(64), grant.fingerprint)
        assertEquals("POST", f.requests.single().method)
        assertEquals("https://accounts.secpar.win/v1/requests/$request/claim", f.requests.single().url)
        assertFalse(grant.toString().contains("aaa"))
    }

    @Test fun `a claim answer that could add a path or a query is refused`() {
        for (relay in listOf("https://relay.example.com/x", "https://relay.example.com?a=b", "http://relay.example.com", "https://a@relay.example.com",
            "https://relay.example.com#f", "wss://relay.example.com", "https://relay.example.com/../v1")) {
            val f = Fake().apply { reply(claimBody.replace("https://rendezvous.example.com", relay)) }
            assertEquals(relay, AccountError.INVALID_RESPONSE, kind { runBlocking { api(f).claimAccess(request) } })
        }
        for (pc in listOf("a/b", "a?b=1", "a&pc=2", "", "x".repeat(65), "a b")) {
            val f = Fake().apply { reply(claimBody.replace("pc_1-A", pc)) }
            assertEquals(pc, AccountError.INVALID_RESPONSE, kind { runBlocking { api(f).claimAccess(request) } })
        }
        val bad = Fake().apply { reply(claimBody.replace("aaa.bbb.ccc", "aaa bbb\"")) }
        assertEquals(AccountError.INVALID_RESPONSE, kind { runBlocking { api(bad).claimAccess(request) } })
    }

    @Test fun `claim errors have their own kinds`() {
        val cases = mapOf("host_not_sharing" to AccountError.HOST_NOT_SHARING, "grant_not_available" to AccountError.GRANT_NOT_AVAILABLE,
            "access_expired_or_revoked" to AccountError.ACCESS_ENDED)
        for ((code, expected) in cases) {
            val f = Fake().apply { reply("""{"error":"$code"}""", if (code.startsWith("access")) 403 else 409) }
            assertEquals(code, expected, kind { runBlocking { api(f).claimAccess(request) } })
        }
        assertEquals("This computer is not letting friends connect right now.", AccountException(AccountError.HOST_NOT_SHARING).message)
    }

    @Test fun `a request can be cancelled through revoke`() = runBlocking {
        val f = Fake().apply { reply("""{"state":"revoked"}""") }
        api(f).cancelAccess(request)
        assertEquals("https://accounts.secpar.win/v1/requests/$request/revoke", f.requests.single().url)
        assertEquals("POST", f.requests.single().method)
    }

    // ---- transport rules ---------------------------------------------------------------------------------

    @Test fun `the transport allows the paths the client uses and nothing else`() {
        val ok = listOf("/auth/device", "/auth/device/token", "/auth/logout", "/v1/me", "/v1/me/visibility", "/v1/computers", "/v1/friends",
            "/v1/blocks", "/v1/directory", "/v1/directory?q=bob_1", "/v1/requests", "/v1/friend-computers", "/v1/owner-pairings",
            "/v1/friends/by-username/bob_1", "/v1/friends/$owner", "/v1/friends/$owner/accept", "/v1/friends/$owner/settings", "/v1/blocks/$owner",
            "/v1/requests/$request", "/v1/requests/$request/claim", "/v1/requests/$request/approve", "/v1/requests/$request/revoke",
            "/v1/owner-pairings/$request")
        for (p in ok) assertTrue(p, transportAllows("https://accounts.secpar.win$p"))
        val no = listOf("/", "/v1", "/v1/friend-computers/x", "/v1/friend-computers?x=1", "/v1/requests?x=1", "/v1/requests/$request/other",
            "/v1/requests/$request/claim/", "/v1/requests/abc/claim", "/v1/requests/../me", "/v1/requests/%2e%2e/claim", "/v1/admin", "/v1/friends//accept",
            "/v1/requests/$request/claim#x", "/v1/friends/by-username/a", "/v1/friends/by-username/Bob")
        for (p in no) assertFalse(p, transportAllows("https://accounts.secpar.win$p"))
        for (u in listOf("http://accounts.secpar.win/v1/me", "https://accounts.secpar.win.evil.test/v1/me", "https://evil.test/v1/me",
            "https://accounts.secpar.win@evil.test/v1/me", "https://accounts.secpar.win:8443/v1/me", "not a url")) assertFalse(u, transportAllows(u))
    }

    @Test fun `the real transport refuses a disallowed path before any network use`() {
        val t = OkHttpAccountTransport()
        for (url in listOf("https://accounts.secpar.win/v1/requests/$request/claim?x=1", "https://accounts.secpar.win/v1/other")) {
            assertEquals(url, AccountError.INVALID_ARGUMENT, kind { t.execute(AccountRequest("POST", url, token, null)) })
        }
    }

    // ---- waiting for the owner -------------------------------------------------------------------------

    private class Status(vararg val steps: Any) : AccountService {
        var i = 0
        val paused = mutableListOf<Long>()
        override suspend fun passwordSignIn(email: String, password: String) = throw UnsupportedOperationException()
        override suspend fun appAuth(route: String, body: Map<String, String>) = throw UnsupportedOperationException()
        override suspend fun me() = throw UnsupportedOperationException()
        override suspend fun computers() = emptyList<OwnedComputer>()
        override suspend fun beginOwnerPair(computerId: String, deviceName: String) = throw UnsupportedOperationException()
        override suspend fun pollOwnerPair(requestId: String) = throw UnsupportedOperationException()
        override suspend fun cancelOwnerPair(requestId: String) {}
        override suspend fun logout() {}
        override suspend fun accessStatus(id: String): AccessRequest {
            val step = steps[minOf(i++, steps.size - 1)]
            if (step is AccountError) throw AccountException(step)
            return step as AccessRequest
        }
    }
    private fun ar(state: String, expires: Long = 2000) = AccessRequest("r", state, expires, listOf("view"))

    @Test fun `waiting polls every 2 seconds until approved`() = runBlocking {
        val s = Status(ar("pending"), ar("pending"), ar("approved"))
        val paused = mutableListOf<Long>()
        val outcome = GuestApproval.wait(s, ar("pending"), { 1000 }) { paused += it }
        assertEquals(Approval.Approved, outcome)
        assertEquals(listOf(2000L, 2000L, 2000L), paused)
    }

    @Test fun `an answer that is already approved needs no poll`() = runBlocking {
        val s = Status()
        assertEquals(Approval.Approved, GuestApproval.wait(s, ar("approved"), { 1000 }) { fail("no pause") })
        assertEquals(0, s.i)
    }

    @Test fun `rejected and revoked end the wait`() = runBlocking {
        for (state in listOf("rejected", "revoked")) {
            val outcome = GuestApproval.wait(Status(ar(state)), ar("pending"), { 1000 }) {}
            assertEquals(Approval.Ended(state), outcome)
        }
    }

    @Test fun `a request past its expiry is dead`() = runBlocking {
        var clock = 1000L
        val outcome = GuestApproval.wait(Status(ar("pending", 1005)), ar("pending", 1005), { clock }) { clock += it / 1000 }
        assertEquals(Approval.Expired, outcome)
        assertEquals(1006L, clock)
    }

    @Test fun `a few network errors do not end the wait but a fourth does`() = runBlocking {
        val net = AccountError.NETWORK
        assertEquals(Approval.Approved, GuestApproval.wait(Status(net, net, net, ar("approved")), ar("pending"), { 1000 }) {})
        assertEquals(AccountError.NETWORK, kind { runBlocking { GuestApproval.wait(Status(net), ar("pending"), { 1000 }) {} } })
    }

    @Test fun `a request the service no longer has counts as ended`() = runBlocking {
        assertEquals(Approval.Ended("revoked"), GuestApproval.wait(Status(AccountError.REQUEST_ANSWERED), ar("pending"), { 1000 }) {})
    }

    // ---- choices ------------------------------------------------------------------------------------------

    @Test fun `the default choices ask for view audio input and controller but not the clipboard`() {
        assertEquals(listOf("view", "audio", "keyboard", "mouse", "gamepad"), GuestChoices().permissions())
        assertEquals(listOf("view", "clipboard"), GuestChoices(audio = false, input = false, controller = false, clipboard = true).permissions())
        assertTrue(GuestChoices(false, false, false, false, false).permissions().isEmpty())
    }

    // ---- the controller flow ------------------------------------------------------------------------------

    private class Memory : KeyValue {
        val values = mutableMapOf<String, String>()
        override fun get(key: String) = values[key]
        override fun put(key: String, value: String?) { if (value == null) values.remove(key) else values[key] = value }
    }
    private object Box : SecretBox {
        override fun seal(plain: String) = Base64.getEncoder().encodeToString(plain.toByteArray())
        override fun open(sealed: String) = String(Base64.getDecoder().decode(sealed))
    }
    private val friendComputer get() = FriendComputer(computer, "Studio PC", "windows", owner, "bob_1")

    private inner class Service(val answers: List<AccessRequest>) : AccountService {
        var asked: List<String>? = null
        var polls = 0
        var claims = 0
        val cancelled = mutableListOf<String>()
        var claimError: AccountError? = null
        var requestError: AccountError? = null
        override suspend fun passwordSignIn(email: String, password: String) = SignInPoll.Authorized(token, 3600)
        override suspend fun appAuth(route: String, body: Map<String, String>) {}
        override suspend fun me() = AccountUser(me, "me_1")
        override suspend fun computers() = emptyList<OwnedComputer>()
        override suspend fun beginOwnerPair(computerId: String, deviceName: String) = throw UnsupportedOperationException()
        override suspend fun pollOwnerPair(requestId: String) = throw UnsupportedOperationException()
        override suspend fun cancelOwnerPair(requestId: String) {}
        override suspend fun logout() {}
        override suspend fun friendComputers() = listOf(friendComputer)
        override suspend fun requestAccess(computer: String, permissions: List<String>): AccessRequest {
            requestError?.let { throw AccountException(it) }
            asked = permissions
            return answers.first()
        }
        override suspend fun accessStatus(id: String) = answers[minOf(++polls, answers.size - 1)]
        override suspend fun claimAccess(id: String): GuestGrant {
            claimError?.let { throw AccountException(it) }
            claims++
            return GuestGrant("g.r.t", 1060, "pc1", "https://relay.example.com", null)
        }
        override suspend fun cancelAccess(id: String) { cancelled += id }
    }

    private class Opened(val grant: GuestGrant, val permissions: Set<String>, val id: String)

    @OptIn(ExperimentalCoroutinesApi::class)
    private fun TestScope.signedIn(service: AccountService, open: (suspend (GuestGrant, FriendComputer, Set<String>, String) -> String)?): AccountController {
        val store = AccountStore(Memory(), Box) { 1000 + testScheduler.currentTime / 1000 }
        val pc = PairedPc("p", "P", listOf("192.0.2.2"), 47999, "f".repeat(64), "t", emptyList())
        val c = AccountController(store, backgroundScope, "Test", { service }, { pc }, kotlinx.coroutines.test.StandardTestDispatcher(testScheduler),
            { 1000 + testScheduler.currentTime / 1000 }, { testScheduler.currentTime }, open)
        runCurrent(); c.signIn("me@example.com", "correct horse"); runCurrent()
        assertNotNull(c.state.value.user)
        return c
    }

    @OptIn(ExperimentalCoroutinesApi::class)
    @Test fun `refresh lists the friends' computers`() = runTest {
        val c = signedIn(Service(listOf(AccessRequest(request, "pending", 9999, emptyList()))), null)
        assertEquals(listOf(friendComputer), c.state.value.friendComputers)
    }

    @OptIn(ExperimentalCoroutinesApi::class)
    @Test fun `connect asks waits claims joins and hands over the route`() = runTest {
        val service = Service(listOf(AccessRequest(request, "pending", 1400, listOf("view", "gamepad")), AccessRequest(request, "pending", 1400, emptyList()),
            AccessRequest(request, "approved", 1400, emptyList())))
        val opened = mutableListOf<Opened>()
        val c = signedIn(service) { grant, _, permissions, id -> opened += Opened(grant, permissions, id); "guest:$id" }
        var route: String? = null
        c.connectFriend(friendComputer, listOf("view", "gamepad", "keyboard")) { route = it }
        runCurrent()
        assertEquals(GuestPhase.WAITING, c.state.value.guest?.phase)
        assertEquals("Waiting for @bob_1", FriendsConnect.phaseText(c.state.value.guest!!))
        assertEquals(listOf("view", "gamepad", "keyboard"), service.asked)
        advanceTimeBy(1900); runCurrent()
        assertEquals(0, service.polls)
        advanceTimeBy(2200); runCurrent()
        assertEquals(2, service.polls)
        assertEquals("guest:$request", route)
        assertEquals(1, service.claims)
        assertEquals(setOf("view", "gamepad"), opened.single().permissions)
        assertNull(c.state.value.guest)
        assertNull(c.state.value.error)
        assertTrue(service.cancelled.isEmpty())
    }

    @OptIn(ExperimentalCoroutinesApi::class)
    @Test fun `an owner who says no ends it in plain words and leaves nothing to cancel`() = runTest {
        val service = Service(listOf(AccessRequest(request, "pending", 1400, listOf("view")), AccessRequest(request, "revoked", 1400, emptyList())))
        val c = signedIn(service) { _, _, _, id -> "guest:$id" }
        var route: String? = null
        c.connectFriend(friendComputer, listOf("view")) { route = it }
        runCurrent(); advanceTimeBy(2100); runCurrent()
        assertNull(route)
        assertEquals("@bob_1 did not allow it.", c.state.value.error)
        assertNull(c.state.value.guest)
        assertEquals(0, service.claims)
        assertTrue(service.cancelled.isEmpty())
    }

    @OptIn(ExperimentalCoroutinesApi::class)
    @Test fun `a request nobody answers stops at its expiry and is cancelled`() = runTest {
        val service = Service(listOf(AccessRequest(request, "pending", 1010, listOf("view"))))
        val c = signedIn(service) { _, _, _, id -> "guest:$id" }
        c.connectFriend(friendComputer, listOf("view")) { fail("no route") }
        runCurrent(); advanceTimeBy(30_000); runCurrent()
        assertEquals("@bob_1 did not answer.", c.state.value.error)
        assertTrue("stops polling", service.polls <= 6)
        assertEquals(listOf(request), service.cancelled)
        assertNull(c.state.value.guest)
    }

    @OptIn(ExperimentalCoroutinesApi::class)
    @Test fun `a host that stopped sharing is reported and the approved request is cancelled`() = runTest {
        val service = Service(listOf(AccessRequest(request, "approved", 1400, listOf("view")))).apply { claimError = AccountError.HOST_NOT_SHARING }
        val c = signedIn(service) { _, _, _, id -> "guest:$id" }
        c.connectFriend(friendComputer, listOf("view")) { fail("no route") }
        runCurrent()
        assertEquals("This computer is not letting friends connect right now.", c.state.value.error)
        assertEquals(listOf(request), service.cancelled)
    }

    @OptIn(ExperimentalCoroutinesApi::class)
    @Test fun `a view only owner is told to turn off keyboard and mouse`() = runTest {
        val service = Service(listOf(AccessRequest(request, "pending", 1400, emptyList()))).apply { requestError = AccountError.INPUT_NOT_ALLOWED }
        val c = signedIn(service) { _, _, _, id -> "guest:$id" }
        c.connectFriend(friendComputer, listOf("keyboard")) { fail("no route") }
        runCurrent()
        assertEquals("@bob_1 lets you view only. Turn off Keyboard and mouse.", c.state.value.error)
        assertTrue(service.cancelled.isEmpty())
    }

    @OptIn(ExperimentalCoroutinesApi::class)
    @Test fun `cancel stops the wait and cancels the request`() = runTest {
        val service = Service(listOf(AccessRequest(request, "pending", 1400, listOf("view"))))
        val c = signedIn(service) { _, _, _, id -> "guest:$id" }
        c.connectFriend(friendComputer, listOf("view")) { fail("no route") }
        runCurrent(); advanceTimeBy(2100); runCurrent()
        c.cancelFriendConnection()
        runCurrent(); advanceTimeBy(10_000); runCurrent()
        assertNull(c.state.value.guest)
        assertEquals(listOf(request), service.cancelled)
        assertEquals(1, service.polls)
        assertNull(c.state.value.error)
    }

    @OptIn(ExperimentalCoroutinesApi::class)
    @Test fun `only one friend request runs at a time`() = runTest {
        val service = Service(listOf(AccessRequest(request, "pending", 1400, listOf("view"))))
        val c = signedIn(service) { _, _, _, id -> "guest:$id" }
        c.connectFriend(friendComputer, listOf("view")) {}
        runCurrent()
        service.asked = null
        c.connectFriend(friendComputer, listOf("view", "audio")) {}
        runCurrent()
        assertNull(service.asked)
        c.cancelFriendConnection(); runCurrent()
    }
}
