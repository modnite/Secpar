package com.pcremote.account

import androidx.compose.ui.graphics.asImageBitmap
import com.pcremote.agent.PairUri
import com.pcremote.agent.PairedPc
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

data class AccountUi(
    val ready: Boolean = false,
    val user: AccountUser? = null,
    val busy: Boolean = false,
    val computers: List<OwnedComputer> = emptyList(),
    val connecting: String? = null,
    val error: String? = null,
    val friends: List<Friend> = emptyList(),
    val blocked: List<AccountUser> = emptyList(),
    /** A short confirmation for the Friends page, such as "Friend request sent". */
    val note: String? = null,
    /** A friend change is in flight. */
    val working: Boolean = false,
    /** Registered users who chose to appear in search, for the current search text. */
    val people: List<DirectoryEntry> = emptyList(),
    /** Connection requests for this account's computers and for this account's own attempts. */
    val requests: List<ConnectionRequest> = emptyList(),
    /** Computers of accepted friends that are online and let friends connect. */
    val friendComputers: List<FriendComputer> = emptyList(),
    /** The request to use a friend's computer that is in progress. */
    val guest: GuestProgress? = null,
)

/** Wording shared with the desktop client. */
object AccountText {
    const val SIGNED_OUT_OFFLINE = "Signed out here. Could not revoke the remote session while offline."
    const val REQUEST_SENT = "Friend request sent"
    const val NO_BROWSER = "No browser is available. Open accounts.secpar.win/activate and enter the code."
    val UUID_ID = Regex("[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}")
    private val USERNAME = Regex("[a-z0-9_]{3,30}")

    /** A username as the server expects it: trimmed, one leading @ removed, lower case. Null when it is not valid. */
    fun cleanUsername(raw: String): String? = raw.trim().removePrefix("@").lowercase().takeIf { USERNAME.matches(it) }
}

/** Main-dispatcher controller. Every asynchronous operation belongs to one login generation. */
class AccountController(
    private val store: AccountStore,
    private val scope: CoroutineScope,
    private val deviceName: String,
    private val apiFactory: (String?) -> AccountService = { AccountApi(it) },
    private val pair: suspend (PairUri) -> PairedPc,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val now: () -> Long = { System.currentTimeMillis() / 1000 },
    private val elapsed: () -> Long = { System.nanoTime() / 1_000_000 },
    /** Joins a friend's computer with the claimed grant and returns the route of the session. Null disables Connect. */
    private val guestOpen: (suspend (GuestGrant, FriendComputer, Set<String>, String) -> String)? = null,
) {
    private val mutable = MutableStateFlow(AccountUi())
    val state: StateFlow<AccountUi> = mutable
    private val thumbs = MutableStateFlow<Map<String, androidx.compose.ui.graphics.ImageBitmap>>(emptyMap())
    /** Live pictures for the cards of friends who let you connect at any time, by computer id. */
    val friendThumbs: StateFlow<Map<String, androidx.compose.ui.graphics.ImageBitmap>> = thumbs
    private val previewJobs = mutableMapOf<String, Job>()
    private val changes = MutableStateFlow(0L)
    val revision: StateFlow<Long> = changes
    private val transition = Mutex()
    private var generation = 0L
    private var session: AccountSession? = null
    private var api: AccountService? = null
    private var loginJob: Job? = null
    private var refreshJob: Job? = null
    private var pairJob: Job? = null
    private var pairGeneration = 0L
    private var expiryJob: Job? = null
    private var guestJob: Job? = null
    private var guestGeneration = 0L

    init {
        scope.launch {
            val saved = withContext(io) { store.session() }
            session = saved
            api = saved?.let { apiFactory(it.token) }
            mutable.value = AccountUi(ready = true, user = saved?.user)
            changes.value++
            saved?.let { scheduleExpiry(it) }
            refresh()
        }
    }

    /**
     * Signs in with an email and password. [done] gets null on success, or the words to show.
     */
    fun signIn(email: String, password: String, done: (String?) -> Unit = {}) {
        if (!state.value.ready || state.value.busy || session != null) return
        val epoch = ++generation
        mutable.value = state.value.copy(busy = true, error = null)
        loginJob = scope.launch {
            val service = apiFactory(null)
            var authorized = false
            var adopted = false
            try {
                val result = service.passwordSignIn(email.trim(), password)
                if (epoch != generation) return@launch
                authorized = true
                val user = service.me()
                val saved = AccountSession(user, result.token, now() + minOf(result.expiresIn, 30L * 86400))
                // A cancel/sign-out during Keystore I/O must not leave a persisted login.
                withContext(NonCancellable) {
                    transition.withLock {
                        if (epoch == generation) {
                            withContext(io) { store.saveSession(saved) }
                            if (epoch != generation) withContext(io) { store.clearSession(saved) }
                            else {
                                session = saved; api = service; adopted = true
                                mutable.value = AccountUi(ready = true, user = user)
                                changes.value++
                                scheduleExpiry(saved)
                            }
                        }
                    }
                }
                if (adopted) { done(null); refresh() }
            } catch (e: CancellationException) { throw e }
            catch (e: Exception) {
                if (epoch == generation) {
                    mutable.value = state.value.copy(busy = false, error = null)
                    done(message(e))
                }
            } finally {
                if (authorized && !adopted) withContext(NonCancellable) { runCatching { service.logout() } }
            }
        }
    }

    /** Register, resend, verify, forgot and reset. [done] gets null on success, or the words to show. */
    fun auth(route: String, body: Map<String, String>, done: (String?) -> Unit) {
        if (!state.value.ready || state.value.busy || session != null) return
        val epoch = generation
        mutable.value = state.value.copy(busy = true, error = null)
        scope.launch {
            try {
                apiFactory(null).appAuth(route, body)
                if (epoch == generation) { mutable.value = state.value.copy(busy = false); done(null) }
            } catch (e: CancellationException) { throw e }
            catch (e: Exception) {
                if (epoch == generation) { mutable.value = state.value.copy(busy = false); done(message(e)) }
            }
        }
    }

    fun cancelSignIn() {
        if (session != null) return
        generation++
        loginJob?.cancel()
        mutable.value = state.value.copy(busy = false, error = null)
    }

    fun signOut() {
        if (!state.value.ready) return
        val service = api
        val epoch = ++generation
        loginJob?.cancel(); refreshJob?.cancel(); pairJob?.cancel(); expiryJob?.cancel()
        guestGeneration++; guestJob?.cancel()
        stopPreviews()
        session = null; api = null
        mutable.value = AccountUi(ready = true, busy = true)
        changes.value++
        scope.launch {
            val cleared = runCatching {
                transition.withLock { withContext(io) { store.clearSavedSession() } }
            }.isSuccess
            changes.value++ // Readers must also invalidate after durable credential removal.
            if (epoch == generation) mutable.value = AccountUi(ready = true,
                error = if (cleared) null else "Could not erase saved access. Please try signing out again.")
            // This API is detached from any future login. Local sign-out does not depend on network.
            if (service != null) {
                val revoked = runCatching { service.logout() }.isSuccess
                if (!revoked && epoch == generation) mutable.value = state.value.copy(
                    error = AccountText.SIGNED_OUT_OFFLINE)
            }
        }
    }

    fun refresh() {
        val saved = session ?: return
        val service = api ?: return
        if (refreshJob?.isActive == true) return
        val epoch = generation
        refreshJob = scope.launch {
            try {
                val computers = service.computers()
                val friends = runCatching { service.friends() }.onFailure { if (it is CancellationException) throw it }.getOrNull()
                val blocked = runCatching { service.blocks() }.onFailure { if (it is CancellationException) throw it }.getOrNull()
                val requests = runCatching { service.requests() }.onFailure { if (it is CancellationException) throw it }.getOrNull()
                val shared = runCatching { service.friendComputers() }.onFailure { if (it is CancellationException) throw it }.getOrNull()
                if (epoch != generation) return@launch
                val removed = withContext(io) {
                    store.owned(saved).filter { access -> computers.none {
                        it.id == access.computerId && it.linked && it.certificateFp == access.pc.fingerprint
                    } }.onEach { store.forgetOwned(saved, it.computerId) }.isNotEmpty()
                }
                if (epoch != generation) return@launch
                mutable.value = state.value.copy(computers = computers, error = null,
                    friends = friends ?: state.value.friends, blocked = blocked ?: state.value.blocked,
                    requests = requests ?: state.value.requests, friendComputers = shared ?: state.value.friendComputers)
                if (removed) changes.value++
                if (shared != null) syncPreviews(service, shared)
            } catch (e: CancellationException) { throw e }
            catch (e: Exception) {
                if (epoch == generation) {
                    if (e is AccountException && e.kind == AccountError.SIGN_IN_REQUIRED) signOut()
                    else mutable.value = state.value.copy(error = message(e))
                }
            }
        }
    }

    fun connect(computer: OwnedComputer, onReady: (String) -> Unit) {
        val saved = session ?: return
        val service = api ?: return
        if (pairJob?.isActive == true || !computer.linked || computer.certificateFp == null) return
        val epoch = generation
        val attempt = ++pairGeneration
        mutable.value = state.value.copy(connecting = computer.id, error = null)
        pairJob = scope.launch {
            var requestId: String? = null
            try {
                val cached = withContext(io) { store.owned(saved).firstOrNull {
                    it.computerId == computer.id && it.pc.fingerprint == computer.certificateFp
                } }
                if (epoch != generation || attempt != pairGeneration) return@launch
                if (cached != null) { onReady(OwnedPcAccess.routeFor(cached.computerId)); return@launch }
                withTimeout(125_000) {
                    val pending = service.beginOwnerPair(computer.id, deviceName.take(80))
                    requestId = pending.id
                    while (now() < pending.expires) {
                        delay(2000)
                        when (val result = service.pollOwnerPair(pending.id)) {
                            is OwnerPairPoll.Pending -> Unit
                            is OwnerPairPoll.Ready -> {
                                requestId = null // Server consumed the single-use result.
                                if (epoch != generation || attempt != pairGeneration) return@withTimeout
                                if (result.expires <= now() || result.certificateFp != computer.certificateFp ||
                                    result.pairing.fingerprint != computer.certificateFp)
                                    throw AccountException(AccountError.INVALID_RESPONSE)
                                val pc = pair(result.pairing)
                                if (epoch != generation || attempt != pairGeneration) return@withTimeout
                                if (pc.fingerprint != computer.certificateFp ||
                                    (result.pairing.pcId != null && pc.id != result.pairing.pcId))
                                    throw AccountException(AccountError.INVALID_RESPONSE)
                                val committed = withContext(io) { store.saveOwned(saved, OwnedAccess(computer.id, pc)) }
                                if (epoch == generation && attempt == pairGeneration && committed) { changes.value++; onReady(OwnedPcAccess.routeFor(computer.id)) }
                                return@withTimeout
                            }
                        }
                    }
                    throw AccountException(AccountError.UNAVAILABLE)
                }
            } catch (e: TimeoutCancellationException) {
                if (epoch == generation && attempt == pairGeneration) mutable.value = state.value.copy(error = "The computer did not respond. Please try again.")
            } catch (e: CancellationException) { throw e }
            catch (e: Exception) {
                if (epoch == generation && attempt == pairGeneration) mutable.value = state.value.copy(error = message(e))
            } finally {
                if (epoch == generation && attempt == pairGeneration) mutable.value = state.value.copy(connecting = null)
                requestId?.let { id -> withContext(NonCancellable) { runCatching { service.cancelOwnerPair(id) } } }
            }
        }
    }

    /**
     * Asks to use a friend's computer, waits for the owner (every 2 seconds, until the answer or the expiry),
     * claims the one-time grant, joins the computer and calls [onReady] with the route of the session.
     */
    fun connectFriend(computer: FriendComputer, permissions: List<String>, onReady: (String) -> Unit) {
        val service = api ?: return
        val open = guestOpen ?: return
        if (session == null || guestJob?.isActive == true) return
        val epoch = generation
        val attempt = ++guestGeneration
        fun current() = epoch == generation && attempt == guestGeneration
        fun phase(p: GuestPhase) { if (current()) mutable.value = state.value.copy(guest = GuestProgress(computer.id, computer.ownerHandle, p)) }
        mutable.value = state.value.copy(guest = GuestProgress(computer.id, computer.ownerHandle, GuestPhase.ASKING), error = null)
        guestJob = scope.launch {
            var cancelable = true // the request is still ours to cancel
            var requestId: String? = null
            var failure: String? = null
            try {
                val first = service.requestAccess(computer.id, permissions)
                requestId = first.id
                if (!current()) return@launch
                phase(GuestPhase.WAITING)
                when (GuestApproval.wait(service, first, now)) {
                    Approval.Approved -> Unit
                    is Approval.Ended -> { cancelable = false; failure = FriendsConnect.declined(computer.ownerHandle); return@launch }
                    Approval.Expired -> { failure = FriendsConnect.noAnswer(computer.ownerHandle); return@launch }
                }
                if (!current()) return@launch
                phase(GuestPhase.OPENING)
                val grant = service.claimAccess(first.id)
                val route = open(grant, computer, first.permissions.toSet(), first.id)
                cancelable = false
                if (current()) onReady(route)
                else { com.pcremote.guest.GuestSessions.find(route)?.conn?.close(); com.pcremote.guest.GuestSessions.remove(route) }
            } catch (e: CancellationException) { throw e }
            catch (e: Exception) {
                failure = if (e is AccountException && e.kind == AccountError.INPUT_NOT_ALLOWED) FriendsConnect.viewOnly(computer.ownerHandle) else message(e)
                if (e is AccountException && e.kind == AccountError.SIGN_IN_REQUIRED && current()) signOut()
            } finally {
                if (current()) mutable.value = state.value.copy(guest = null, error = failure ?: state.value.error)
                // A request nobody answered, or one we could not use, would stay open for the owner to allow later.
                requestId?.takeIf { cancelable }?.let { id -> withContext(NonCancellable) { runCatching { service.cancelAccess(id) } } }
            }
        }
    }

    /** Starts a preview for each friend computer that may be watched without asking, and stops the others. */
    private fun syncPreviews(service: AccountService, computers: List<FriendComputer>) {
        val open = guestOpen ?: return
        val wanted = computers.filter { it.connect == "always" }
        previewJobs.keys.filter { id -> wanted.none { it.id == id } }.forEach { id -> previewJobs.remove(id)?.cancel(); thumbs.value = thumbs.value - id }
        wanted.forEach { c ->
            if (previewJobs[c.id]?.isActive != true) previewJobs[c.id] = scope.launch { watchPreview(service, open, c) }
        }
    }

    private fun stopPreviews() {
        previewJobs.values.forEach { it.cancel() }
        previewJobs.clear()
        thumbs.value = emptyMap()
    }

    /**
     * One friend computer's preview: a view-only session that subscribes to thumbnails. The host ends it when a
     * real connection comes, and the loop starts again after a pause that grows while nothing works.
     */
    private suspend fun watchPreview(service: AccountService, open: suspend (GuestGrant, FriendComputer, Set<String>, String) -> String, c: FriendComputer) {
        var wait = 10_000L
        while (true) {
            val started = elapsed()
            var route: String? = null
            try {
                val first = service.requestAccess(c.id, listOf("view"))
                if (first.approved) {
                    route = open(service.claimAccess(first.id), c, first.permissions.toSet(), first.id)
                    val conn = com.pcremote.guest.GuestSessions.find(route)?.conn
                    if (conn != null && (conn.state.value as? com.pcremote.agent.AgentState.Online)?.caps?.contains("thumb") == true) {
                        conn.send(com.pcremote.agent.Msg.ThumbSub(true))
                        coroutineScope {
                            val reader = launch {
                                conn.binary.collect { b ->
                                    if (b.isNotEmpty() && b[0].toInt() == 2)
                                        android.graphics.BitmapFactory.decodeByteArray(b, 1, b.size - 1)?.let { thumbs.value = thumbs.value + (c.id to it.asImageBitmap()) }
                                }
                            }
                            conn.state.first { it !is com.pcremote.agent.AgentState.Online }
                            reader.cancel()
                        }
                    } else wait = 300_000L // The computer gives no thumbnails.
                } else wait = 300_000L // The owner now wants to be asked.
            } catch (e: CancellationException) { throw e }
            catch (e: Exception) {
                if (e is AccountException && e.kind == AccountError.SIGN_IN_REQUIRED) return
            } finally {
                thumbs.value = thumbs.value - c.id
                route?.let { com.pcremote.guest.GuestSessions.find(it)?.conn?.close(); com.pcremote.guest.GuestSessions.remove(it) }
            }
            if (elapsed() - started > 60_000) wait = 10_000L
            delay(wait)
            wait = minOf(300_000L, wait * 2)
        }
    }

    fun cancelFriendConnection() {
        guestGeneration++
        guestJob?.cancel()
        mutable.value = state.value.copy(guest = null)
    }

    fun dismissError() { mutable.value = state.value.copy(error = null) }

    fun showNote(text: String) { mutable.value = state.value.copy(note = text) }

    /** Add by username, accept a request, or change the lists. Refreshes the lists afterwards. */
    fun friendAction(action: FriendAction, rawId: String) {
        val service = api ?: return
        if (session == null || state.value.working) return
        val id = if (action == FriendAction.INVITE) AccountText.cleanUsername(rawId) else rawId.trim().lowercase().takeIf { AccountText.UUID_ID.matches(it) }
        if (id == null) { mutable.value = state.value.copy(error = AccountException(AccountError.BAD_NAME).message); return }
        val epoch = generation
        mutable.value = state.value.copy(working = true, error = null, note = null)
        scope.launch {
            try {
                service.friendAction(action, id)
                val friends = service.friends()
                val blocked = service.blocks()
                if (epoch != generation) return@launch
                mutable.value = state.value.copy(friends = friends, blocked = blocked, working = false,
                    note = if (action == FriendAction.INVITE) AccountText.REQUEST_SENT else null)
            } catch (e: CancellationException) { throw e }
            catch (e: Exception) {
                if (epoch != generation) return@launch
                if (e is AccountException && e.kind == AccountError.SIGN_IN_REQUIRED) signOut()
                else mutable.value = state.value.copy(working = false, error = message(e))
            }
        }
    }

    /** The directory of users who chose to appear. An empty [query] lists the first names. */
    fun search(query: String) {
        val service = api ?: return
        if (session == null) return
        val epoch = generation
        scope.launch {
            try {
                val people = service.directory(query)
                if (epoch == generation) mutable.value = state.value.copy(people = people)
            } catch (e: CancellationException) { throw e }
            catch (e: Exception) {
                if (epoch != generation) return@launch
                if (e is AccountException && e.kind == AccountError.SIGN_IN_REQUIRED) signOut()
                else mutable.value = state.value.copy(people = emptyList(), error = message(e))
            }
        }
    }

    fun setVisible(visible: Boolean) {
        val service = api ?: return
        val user = state.value.user ?: return
        if (session == null) return
        val epoch = generation
        mutable.value = state.value.copy(user = user.copy(visible = visible), error = null)
        scope.launch {
            try { service.setVisible(visible) }
            catch (e: CancellationException) { throw e }
            catch (e: Exception) {
                if (epoch != generation) return@launch
                if (e is AccountException && e.kind == AccountError.SIGN_IN_REQUIRED) signOut()
                else mutable.value = state.value.copy(user = state.value.user?.copy(visible = !visible), error = message(e))
            }
        }
    }

    /** Allows or blocks keyboard and mouse for one friend, each on its own. */
    fun friendInput(id: String, keyboard: Boolean?, mouse: Boolean?) {
        val service = api ?: return
        if (session == null || state.value.working) return
        val epoch = generation
        mutable.value = state.value.copy(working = true, error = null, note = null)
        scope.launch {
            try {
                service.friendInput(id, keyboard, mouse)
                val friends = service.friends()
                if (epoch == generation) mutable.value = state.value.copy(friends = friends, working = false)
            } catch (e: CancellationException) { throw e }
            catch (e: Exception) {
                if (epoch != generation) return@launch
                if (e is AccountException && e.kind == AccountError.SIGN_IN_REQUIRED) signOut()
                else mutable.value = state.value.copy(working = false, error = message(e))
            }
        }
    }

    /** Keyboard and mouse ("view" or "full") and connect policy ("ask" or "always") for one friend. */
    fun friendSetting(id: String, input: String?, connect: String?) {
        val service = api ?: return
        if (session == null || state.value.working) return
        val epoch = generation
        mutable.value = state.value.copy(working = true, error = null, note = null)
        scope.launch {
            try {
                service.friendSetting(id, input, connect)
                val friends = service.friends()
                if (epoch == generation) mutable.value = state.value.copy(friends = friends, working = false)
            } catch (e: CancellationException) { throw e }
            catch (e: Exception) {
                if (epoch != generation) return@launch
                if (e is AccountException && e.kind == AccountError.SIGN_IN_REQUIRED) signOut()
                else mutable.value = state.value.copy(working = false, error = message(e))
            }
        }
    }

    /** Allow or deny a connection request, or accept or decline a friend. Refreshes the lists afterwards. */
    fun answerRequest(answer: Answer, id: String) {
        if (session == null || state.value.working) return
        mutable.value = state.value.copy(working = true, error = null, note = null)
        scope.launch {
            val ok = answerNow(answer, id)
            mutable.value = state.value.copy(working = false, note = if (ok) null else state.value.note)
        }
    }

    /** Waits for the saved login to load, answers, then reloads the lists. False when it failed. */
    suspend fun answerNow(answer: Answer, id: String): Boolean {
        state.first { it.ready }
        val service = api ?: return false
        val epoch = generation
        return try {
            service.answer(answer, id)
            val friends = runCatching { service.friends() }.getOrNull()
            val requests = runCatching { service.requests() }.getOrNull()
            if (epoch == generation) mutable.value = state.value.copy(friends = friends ?: state.value.friends,
                requests = requests ?: state.value.requests)
            true
        } catch (e: CancellationException) { throw e }
        catch (e: Exception) {
            if (epoch == generation) {
                if (e is AccountException && e.kind == AccountError.SIGN_IN_REQUIRED) signOut()
                else mutable.value = state.value.copy(error = message(e))
            }
            false
        }
    }

    /** Friends and connection requests as the account service has them now, or null when signed out or offline. */
    suspend fun poll(): Pair<List<Friend>, List<ConnectionRequest>>? {
        state.first { it.ready }
        val service = api ?: return null
        val epoch = generation
        return try {
            val friends = service.friends()
            val requests = service.requests()
            if (epoch == generation) mutable.value = state.value.copy(friends = friends, requests = requests)
            friends to requests
        } catch (e: CancellationException) { throw e }
        catch (e: Exception) {
            if (epoch == generation && e is AccountException && e.kind == AccountError.SIGN_IN_REQUIRED) signOut()
            null
        }
    }

    fun cancelConnection() {
        pairGeneration++
        pairJob?.cancel()
        mutable.value = state.value.copy(connecting = null)
    }

    private fun scheduleExpiry(saved: AccountSession) {
        expiryJob?.cancel()
        expiryJob = scope.launch {
            delay((saved.expiresAt - now()).coerceAtLeast(0) * 1000)
            if (session?.token == saved.token) signOut()
        }
    }

    private fun message(e: Exception) = (e as? AccountException)?.message ?: (e as? com.pcremote.guest.GuestException)?.message
        ?: "Could not complete the connection. Please try again."
}
