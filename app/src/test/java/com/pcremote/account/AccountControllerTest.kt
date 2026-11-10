package com.pcremote.account

import com.pcremote.agent.*
import kotlinx.coroutines.*
import kotlinx.coroutines.test.*
import org.junit.Assert.*
import org.junit.Test
import java.util.Base64

@OptIn(ExperimentalCoroutinesApi::class)
class AccountControllerTest {
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
    private val computerId = "22222222-2222-4222-8222-222222222222"
    private val request = "33333333-3333-4333-8333-333333333333"
    private val token = "a".repeat(64)
    private val fp = "f".repeat(64)
    private val user get() = AccountUser(owner, "review-user")
    private val computer get() = OwnedComputer(computerId, "Review PC", "windows", fp, true, true)
    private val pc get() = PairedPc("host-id", "Review PC", listOf("192.0.2.2"), 47999, fp, "device-token", emptyList())
    private val uri get() = PairUri(pc.name, pc.hosts, pc.port, fp, "fixture-only", pcId = pc.id)

    private inner class Fake : AccountService {
        var entries = listOf(computer)
        var signPoll: SignInPoll.Authorized = SignInPoll.Authorized(token, 3600)
        var ownerPoll: OwnerPairPoll = OwnerPairPoll.Ready(uri, fp, 1120)
        var logoutFails = false
        var loggedOut = false
        var ownerRequests = 0
        var cancelled = 0
        var meGate: CompletableDeferred<Unit>? = null
        var friendList = listOf<Friend>()
        var blockList = listOf<AccountUser>()
        val actions = mutableListOf<Pair<FriendAction, String>>()
        var actionFails: AccountError? = null
        override suspend fun passwordSignIn(email: String, password: String) = signPoll
        override suspend fun appAuth(route: String, body: Map<String, String>) {}
        override suspend fun me(): AccountUser {
            meGate?.let { withContext(NonCancellable) { it.await() } }
            return user
        }
        override suspend fun computers() = entries
        override suspend fun friends() = friendList
        override suspend fun blocks() = blockList
        override suspend fun friendAction(action: FriendAction, id: String) {
            actionFails?.let { throw AccountException(it) }
            actions += action to id
            if (action == FriendAction.INVITE) friendList = friendList + Friend(id, "new-friend", "pending", owner)
        }
        override suspend fun beginOwnerPair(computerId: String, deviceName: String): OwnerPairPending {
            ownerRequests++
            return OwnerPairPending(request, 1120)
        }
        override suspend fun pollOwnerPair(requestId: String) = ownerPoll
        override suspend fun cancelOwnerPair(requestId: String) { cancelled++ }
        override suspend fun logout() {
            loggedOut = true
            if (logoutFails) throw AccountException(AccountError.NETWORK)
        }
    }
    private fun TestScope.store(kv: Memory = Memory()) = AccountStore(kv, Box) { 1000 + testScheduler.currentTime / 1000 }
    private fun TestScope.controller(store: AccountStore, api: (String?) -> AccountService,
        pair: suspend (PairUri) -> PairedPc = { pc }) = AccountController(store, backgroundScope, "Review Android",
        api, pair, StandardTestDispatcher(testScheduler), { 1000 + testScheduler.currentTime / 1000 }, { testScheduler.currentTime })
    private fun TestScope.login(c: AccountController) {
        runCurrent(); c.signIn("me@example.com", "correct horse"); runCurrent()
        assertEquals(user, c.state.value.user)
    }

    @Test fun `browser login and owned pairing persist separate credentials`() = runTest {
        val store = store()
        val fake = Fake()
        val c = controller(store, { fake })
        login(c)
        assertEquals(token, store.session()!!.token)
        assertEquals(listOf(computer), c.state.value.computers)
        var opened: String? = null
        c.connect(computer) { opened = it }
        runCurrent(); advanceTimeBy(2000); runCurrent()
        assertEquals(OwnedPcAccess.routeFor(computerId), opened)
        assertEquals(pc.token, store.owned(store.session()!!).single().pc.token)
        assertNull(c.state.value.connecting)
        assertNull(c.state.value.error)
    }

    @Test fun `bootstrap fingerprint must match enrolled cloud computer before any host traffic`() = runTest {
        val store = store()
        val fake = Fake().apply { ownerPoll = OwnerPairPoll.Ready(uri, "b".repeat(64), 1120) }
        var hostCalls = 0
        val c = controller(store, { fake }, { hostCalls++; pc })
        login(c)
        c.connect(computer) { fail("Must not open") }
        runCurrent(); advanceTimeBy(2000); runCurrent()
        assertEquals(0, hostCalls)
        assertTrue(store.owned(store.session()!!).isEmpty())
        assertNotNull(c.state.value.error)
    }

    @Test fun `late login after cancellation cannot replace a newer login`() = runTest {
        val store = store()
        val gate = CompletableDeferred<Unit>()
        val old = Fake().apply { meGate = gate }
        val fresh = Fake().apply { signPoll = SignInPoll.Authorized("b".repeat(64), 3600) }
        val services = ArrayDeque(listOf(old, fresh))
        val c = controller(store, { services.removeFirst() })
        runCurrent(); c.signIn("me@example.com", "correct horse"); runCurrent()
        c.cancelSignIn(); c.signIn("me@example.com", "correct horse"); runCurrent()
        assertEquals("b".repeat(64), store.session()!!.token)
        gate.complete(Unit); runCurrent()
        assertEquals("b".repeat(64), store.session()!!.token)
        assertTrue(old.loggedOut)
        assertFalse(fresh.loggedOut)
    }

    @Test fun `late host pairing after sign out cannot restore access or navigate`() = runTest {
        val store = store()
        val gate = CompletableDeferred<Unit>()
        val c = controller(store, { Fake() }, { withContext(NonCancellable) { gate.await() }; pc })
        login(c)
        c.connect(computer) { fail("Stale callback navigated") }
        runCurrent(); advanceTimeBy(2000); runCurrent()
        val before = store.session()!!
        c.signOut(); runCurrent()
        gate.complete(Unit); runCurrent()
        assertNull(c.state.value.user)
        assertNull(store.session())
        assertTrue(store.owned(before).isEmpty())
    }

    @Test fun `saved access opens without creating another pairing request`() = runTest {
        val store = store()
        val fake = Fake()
        val c = controller(store, { fake })
        login(c)
        store.saveOwned(store.session()!!, OwnedAccess(computerId, pc))
        var opened: String? = null
        c.connect(computer) { opened = it }; runCurrent()
        assertEquals(OwnedPcAccess.routeFor(computerId), opened)
        assertEquals(0, fake.ownerRequests)
    }

    @Test fun `sign out clears local access even when online revocation fails`() = runTest {
        val store = store()
        val fake = Fake().apply { logoutFails = true }
        val c = controller(store, { fake })
        login(c)
        store.saveOwned(store.session()!!, OwnedAccess(computerId, pc))
        c.signOut(); runCurrent()
        assertNull(store.session())
        assertNull(c.state.value.user)
        assertFalse(c.state.value.busy)
        assertTrue(c.state.value.error!!.contains("Could not revoke"))
    }

    @Test fun `restoring login preserves cached access but unlinked computers revoke it`() = runTest {
        val store = store()
        val saved = AccountSession(user, token, 4600)
        store.saveSession(saved); store.saveOwned(saved, OwnedAccess(computerId, pc))
        val fake = Fake()
        val c = controller(store, { fake }); runCurrent()
        assertEquals(user, c.state.value.user)
        assertEquals(1, store.owned(saved).size)
        val revision = c.revision.value
        fake.entries = listOf(computer.copy(linked = false, certificateFp = null))
        c.refresh(); runCurrent()
        assertTrue(store.owned(saved).isEmpty())
        assertTrue(c.revision.value > revision)
    }

    @Test fun `cancel removes pending request and clears connection state`() = runTest {
        val fake = Fake().apply { ownerPoll = OwnerPairPoll.Pending(request, 1120) }
        val c = controller(store(), { fake })
        login(c)
        c.connect(computer) { fail("Must not open") }; runCurrent()
        c.cancelConnection(); runCurrent()
        assertEquals(1, fake.cancelled)
        assertNull(c.state.value.connecting)
    }

    @Test fun `cancelled request cleanup cannot clear a new connection attempt`() = runTest {
        val fake = Fake().apply { ownerPoll = OwnerPairPoll.Pending(request, 1120) }
        val c = controller(store(), { fake })
        login(c)
        c.connect(computer) { fail("Must not open") }; runCurrent()
        c.cancelConnection()
        c.connect(computer) { fail("Must not open") }
        runCurrent()
        assertEquals(2, fake.ownerRequests)
        assertEquals(computer.id, c.state.value.connecting)
        c.cancelConnection(); runCurrent()
    }

    @Test fun `native session expiry clears durable access and publishes invalidation`() = runTest {
        val store = store()
        val saved = AccountSession(user, token, 1001)
        store.saveSession(saved); store.saveOwned(saved, OwnedAccess(computerId, pc))
        val fake = Fake()
        val c = controller(store, { fake }); runCurrent()
        val revision = c.revision.value
        advanceTimeBy(1000); runCurrent()
        assertNull(c.state.value.user)
        assertNull(store.session())
        assertTrue(c.revision.value >= revision + 2)
        assertTrue(fake.loggedOut)
    }

    private val friendCode = "44444444-4444-4444-8444-444444444444"

    @Test fun `refresh loads friends and blocked people`() = runTest {
        val fake = Fake().apply {
            friendList = listOf(Friend(friendCode, "river", "accepted", owner))
            blockList = listOf(AccountUser("55555555-5555-4555-8555-555555555555", "noisy"))
        }
        val c = controller(store(), { fake })
        login(c)
        assertEquals("river", c.state.value.friends.single().handle)
        assertEquals("noisy", c.state.value.blocked.single().handle)
    }

    @Test fun `adding a friend sends the cleaned username and shows the sent note`() = runTest {
        val fake = Fake()
        val c = controller(store(), { fake })
        login(c)
        c.friendAction(FriendAction.INVITE, "  @River_Two \n"); runCurrent()
        assertEquals(listOf(FriendAction.INVITE to "river_two"), fake.actions)
        assertEquals("Friend request sent", c.state.value.note)
        assertEquals(1, c.state.value.friends.size)
        assertFalse(c.state.value.working)
    }

    @Test fun `an invalid username is refused without a request`() = runTest {
        val fake = Fake()
        val c = controller(store(), { fake })
        login(c)
        for (bad in listOf("", "  ", "@", "ab", "not a name", "../me", "river-x", friendCode)) {
            c.friendAction(FriendAction.INVITE, bad); runCurrent()
            assertEquals("Enter a username of 3 to 30 letters, numbers or underscores.", c.state.value.error)
            c.dismissError()
        }
        assertTrue(fake.actions.isEmpty())
    }

    @Test fun `accept and block still take account ids`() = runTest {
        val fake = Fake()
        val c = controller(store(), { fake })
        login(c)
        c.friendAction(FriendAction.ACCEPT, friendCode); runCurrent()
        assertEquals(listOf(FriendAction.ACCEPT to friendCode), fake.actions)
        c.friendAction(FriendAction.BLOCK, "river"); runCurrent()
        assertEquals(1, fake.actions.size)
        assertNotNull(c.state.value.error)
    }

    @Test fun `a friend error shows the shared wording and can be dismissed`() = runTest {
        val fake = Fake().apply { actionFails = AccountError.FRIEND_UNAVAILABLE }
        val c = controller(store(), { fake })
        login(c)
        c.friendAction(FriendAction.INVITE, "river"); runCurrent()
        assertEquals("No one with that username can be added.", c.state.value.error)
        assertFalse(c.state.value.working)
        c.dismissError()
        assertNull(c.state.value.error)
    }

    @Test fun `signing out forgets the friend lists`() = runTest {
        val fake = Fake().apply { friendList = listOf(Friend(friendCode, "river", "accepted", owner)) }
        val c = controller(store(), { fake })
        login(c)
        c.signOut(); runCurrent()
        assertTrue(c.state.value.friends.isEmpty())
    }
}
