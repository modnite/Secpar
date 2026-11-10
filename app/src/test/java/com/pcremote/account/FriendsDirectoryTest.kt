package com.pcremote.account

import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class FriendsDirectoryTest {
    private val me = "00000000-0000-4000-8000-000000000001"
    private val other = "00000000-0000-4000-8000-000000000002"
    private val request = "00000000-0000-4000-8000-000000000003"
    private val token = "d".repeat(64)
    private class Fake : AccountTransport {
        val requests = mutableListOf<AccountRequest>()
        val replies = ArrayDeque<AccountResponse>()
        fun reply(body: String, status: Int = 200) { replies.add(AccountResponse(status, body.toByteArray())) }
        override fun execute(request: AccountRequest): AccountResponse { requests.add(request); return replies.removeFirst() }
    }
    private fun api(f: Fake) = AccountApi(token, f)

    @Test fun `search asks the directory with a clean lower case name`() = runBlocking {
        val f = Fake().apply { reply("""[{"handle":"Bob_1","name":"x","cursor":"bob_1","relation":"none"},{"handle":"Cy","cursor":"cy","name":"x","relation":"friend"}]""") }
        val found = api(f).directory(" @Bob_1 ")
        assertEquals(listOf("Bob_1", "Cy"), found.map { it.handle })
        assertEquals("https://accounts.secpar.win/v1/directory?q=bob_1", f.requests.single().url)
        assertEquals("GET", f.requests.single().method)
    }

    @Test fun `text that is not a username never reaches the url`() = runBlocking {
        val f = Fake()
        for (bad in listOf("a/../b", "a b", "x".repeat(31), "b&q=c", "%41")) assertTrue(bad, api(f).directory(bad).isEmpty())
        assertTrue(f.requests.isEmpty())
    }

    @Test fun `an empty search lists the first names`() = runBlocking {
        val f = Fake().apply { reply("[]") }
        api(f).directory("")
        assertEquals("https://accounts.secpar.win/v1/directory", f.requests.single().url)
    }

    @Test fun `the transport only allows a query on the directory`() {
        val t = OkHttpAccountTransport()
        for (url in listOf("https://accounts.secpar.win/v1/friends?q=a", "https://accounts.secpar.win/v1/directory?q=A&x=1",
            "https://accounts.secpar.win/v1/directory?x=1", "https://accounts.secpar.win/v1/directory?q=a%2Fb")) {
            try { t.execute(AccountRequest("GET", url, token, null)); fail(url) }
            catch (e: AccountException) { assertEquals(url, AccountError.INVALID_ARGUMENT, e.kind) }
        }
    }

    @Test fun `a directory answer with a wrong relation or too many names is refused`() = runBlocking {
        val bad = Fake().apply { reply("""[{"handle":"a","name":"x","cursor":"a","relation":"owner"}]""") }
        try { api(bad).directory("a"); fail() } catch (e: AccountException) { assertEquals(AccountError.INVALID_RESPONSE, e.kind) }
        val many = Fake().apply { reply("[" + (1..21).joinToString(",") { """{"handle":"a$it","name":"x","cursor":"a$it","relation":"none"}""" } + "]") }
        try { api(many).directory("a"); fail() } catch (e: AccountException) { assertEquals(AccountError.INVALID_RESPONSE, e.kind) }
    }

    @Test fun `visibility and friend settings are sent as fixed json`() = runBlocking {
        val f = Fake().apply { reply("""{"visible":true}"""); reply("""{"input":"full","connect":"ask"}""") }
        api(f).setVisible(true)
        api(f).friendSetting(other, "full", null)
        assertEquals("PUT https://accounts.secpar.win/v1/me/visibility", f.requests[0].let { "${it.method} ${it.url}" })
        assertEquals("""{"visible":true}""", f.requests[0].body)
        assertEquals("PUT https://accounts.secpar.win/v1/friends/$other/settings", f.requests[1].let { "${it.method} ${it.url}" })
        assertEquals("""{"input":"full"}""", f.requests[1].body)
    }

    @Test fun `keyboard and mouse are sent apart and read apart`() = runBlocking {
        val f = Fake().apply {
            reply("""{"input":"view","connect":"ask","keyboard":true,"mouse":false}""")
            reply("""[{"id":"$other","handle":"bob","state":"accepted","requester":"$me","input":"view","connect":"ask","keyboard":true,"mouse":false},{"id":"$me","handle":"x","state":"accepted","requester":"$me","input":"full","connect":"always"}]""")
        }
        api(f).friendInput(other, true, null)
        assertEquals("""{"keyboard":true}""", f.requests[0].body)
        val list = api(f).friends()
        assertEquals("Ask first · Keyboard only", FriendsModel.summary(list[0]))
        // An older service sends only input, which decides both.
        assertEquals("Any time · Keyboard and mouse", FriendsModel.summary(list[1]))
        try { api(f).friendInput(other, null, null); fail() } catch (e: AccountException) { assertEquals(AccountError.INVALID_ARGUMENT, e.kind) }
    }

    @Test fun `friend settings refuse bad ids and values before any request`() = runBlocking {
        val f = Fake()
        for ((id, input, connect) in listOf(Triple("../me", "full", null), Triple(other, "admin", null), Triple(other, null, "never"), Triple(other, null, null)))
            try { api(f).friendSetting(id, input, connect); fail() } catch (e: AccountException) { assertEquals(AccountError.INVALID_ARGUMENT, e.kind) }
        assertTrue(f.requests.isEmpty())
    }

    @Test fun `friends without settings read as view only and ask first`() = runBlocking {
        val f = Fake().apply { reply("""[{"id":"$other","handle":"bob","state":"accepted","requester":"$me"},{"id":"$me","handle":"x","state":"accepted","requester":"$me","input":"full","connect":"always"}]""") }
        val list = api(f).friends()
        assertEquals("view" to "ask", list[0].input to list[0].connect)
        assertEquals("full" to "always", list[1].input to list[1].connect)
        val broken = Fake().apply { reply("""[{"id":"$other","handle":"bob","state":"accepted","requester":"$me","input":"root"}]""") }
        try { api(broken).friends(); fail() } catch (e: AccountException) { assertEquals(AccountError.INVALID_RESPONSE, e.kind) }
    }

    @Test fun `requests name the guest and the computer`() = runBlocking {
        val f = Fake().apply { reply("""[{"id":"$request","owner":"$me","guest":"$other","guest_handle":"bob","computer_name":"Studio","requested":"[\"view\",\"audio\"]","state":"pending"}]""") }
        val r = api(f).requests().single()
        assertEquals("@bob wants to use Studio", FriendsModel.requestText(r))
        assertEquals(listOf("view", "audio"), r.requested)
        assertEquals(listOf(r), FriendsModel.waiting(listOf(r), me))
        assertTrue(FriendsModel.waiting(listOf(r), other).isEmpty())
    }

    @Test fun `allow reads the request then approves exactly what was asked`() = runBlocking {
        val f = Fake().apply {
            reply("""{"id":"$request","requested":"[\"view\",\"keyboard\"]","state":"pending"}""")
            reply("""{"state":"approved"}""")
        }
        api(f).answer(Answer.ALLOW_CONNECTION, request)
        assertEquals("GET /v1/requests/$request", f.requests[0].let { it.method + " " + it.url.removePrefix("https://accounts.secpar.win") })
        assertEquals("POST", f.requests[1].method)
        assertTrue(f.requests[1].url.endsWith("/v1/requests/$request/approve"))
        assertEquals("""{"permissions":["view","keyboard"]}""", f.requests[1].body)
    }

    @Test fun `deny accept and decline use the fixed routes`() = runBlocking {
        val f = Fake().apply { repeat(3) { reply("{}") } }
        api(f).answer(Answer.DENY_CONNECTION, request)
        api(f).answer(Answer.ACCEPT_FRIEND, other)
        api(f).answer(Answer.DECLINE_FRIEND, other)
        assertEquals(listOf("POST /v1/requests/$request/revoke", "POST /v1/friends/$other/accept", "DELETE /v1/friends/$other"),
            f.requests.map { it.method + " " + it.url.removePrefix("https://accounts.secpar.win") })
        try { api(f).answer(Answer.DENY_CONNECTION, "../../me"); fail() } catch (e: AccountException) { assertEquals(AccountError.INVALID_ARGUMENT, e.kind) }
    }

    @Test fun `server codes become plain messages`() = runBlocking {
        val f = Fake().apply { reply("""{"error":"input_not_allowed"}""", 400); reply("""{"error":"request_not_pending"}""", 409) }
        try { api(f).answer(Answer.ALLOW_CONNECTION.also { }, request); fail() } catch (e: AccountException) { assertEquals(AccountError.INPUT_NOT_ALLOWED, e.kind) }
        f.replies.clear(); f.reply("""{"error":"request_not_pending"}""", 409)
        try { api(f).answer(Answer.DENY_CONNECTION, request); fail() } catch (e: AccountException) { assertEquals(AccountError.REQUEST_ANSWERED, e.kind) }
    }

    @Test fun `summary and search labels use the desktop wording`() {
        fun friend(input: String, connect: String) = Friend(other, "bob", "accepted", me, input, connect)
        assertEquals("Ask first · View only", FriendsModel.summary(friend("view", "ask")))
        assertEquals("Any time · Keyboard and mouse", FriendsModel.summary(friend("full", "always")))
        assertNull(FriendsModel.personLabel(DirectoryEntry("a", "a", "none")))
        assertEquals("Friend", FriendsModel.personLabel(DirectoryEntry("a", "a", "friend")))
        assertEquals("Request sent", FriendsModel.personLabel(DirectoryEntry("a", "a", "requested")))
        assertEquals("Sent you a request", FriendsModel.personLabel(DirectoryEntry("a", "a", "incoming")))
    }

    private fun friend(id: String, state: String, requester: String) = Friend(id, id, state, requester)
    private fun waiting(id: String, owner: String = me, state: String = "pending") =
        ConnectionRequest(id, owner, "bob", "Studio", listOf("view"), state)

    @Test fun `alerts fire once and skip friend requests that were waiting at start`() {
        val tracker = AlertTracker()
        assertTrue(tracker.next(me, listOf(friend("old", "pending", "old")), emptyList()).isEmpty())
        val friends = listOf(friend("old", "pending", "old"), friend("new", "pending", "new"), friend("mine", "pending", me), friend("pal", "accepted", "pal"))
        val got = tracker.next(me, friends, listOf(waiting("r1"), waiting("r2", state = "approved"), waiting("r3", owner = other)))
        assertEquals(listOf("new" to Answer.ACCEPT_FRIEND, "r1" to Answer.ALLOW_CONNECTION), got.map { it.id to it.yes })
        assertEquals(Answer.DENY_CONNECTION, got[1].no)
        assertEquals("@bob wants to use Studio", got[1].text)
        assertTrue(tracker.next(me, friends, listOf(waiting("r1"))).isEmpty())
    }

    @Test fun `a connection request waiting at start is announced`() {
        assertEquals(1, AlertTracker().next(me, emptyList(), listOf(waiting("r1"))).size)
    }

    @Test fun `notification buttons have plain labels`() {
        assertEquals("Allow", AlertService.label(Answer.ALLOW_CONNECTION))
        assertEquals("Deny", AlertService.label(Answer.DENY_CONNECTION))
        assertEquals("Accept", AlertService.label(Answer.ACCEPT_FRIEND))
        assertFalse(AlertService.label(Answer.DECLINE_FRIEND).isBlank())
    }
}
