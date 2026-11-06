package com.pcremote.account

import com.pcremote.agent.PairUri
import com.pcremote.agent.PairedPc
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

data class AccountUi(
    val ready: Boolean = false,
    val user: AccountUser? = null,
    val busy: Boolean = false,
    val approval: DeviceSignIn? = null,
    val computers: List<OwnedComputer> = emptyList(),
    val connecting: String? = null,
    val error: String? = null,
)

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
) {
    private val mutable = MutableStateFlow(AccountUi())
    val state: StateFlow<AccountUi> = mutable
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

    fun signIn() {
        if (!state.value.ready || state.value.busy || session != null) return
        val epoch = ++generation
        mutable.value = state.value.copy(busy = true, error = null)
        loginJob = scope.launch {
            val service = apiFactory(null)
            var authorized = false
            var adopted = false
            try {
                val approval = service.beginSignIn(deviceName.take(60))
                if (epoch != generation) return@launch
                mutable.value = state.value.copy(approval = approval)
                val deadline = elapsed() + approval.expiresIn * 1000
                var interval = maxOf(5, approval.interval)
                while (elapsed() < deadline) {
                    delay(interval * 1000)
                    if (epoch != generation) return@launch
                    when (val result = service.pollSignIn(approval.deviceCode)) {
                        SignInPoll.Pending -> Unit
                        SignInPoll.SlowDown -> interval = minOf(60, interval + 5)
                        is SignInPoll.Authorized -> {
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
                            if (adopted) refresh()
                            return@launch
                        }
                    }
                }
                throw AccountException(AccountError.EXPIRED)
            } catch (e: CancellationException) { throw e }
            catch (e: Exception) {
                if (epoch == generation) mutable.value = state.value.copy(busy = false, approval = null, error = message(e))
            } finally {
                if (authorized && !adopted) withContext(NonCancellable) { runCatching { service.logout() } }
            }
        }
    }

    fun cancelSignIn() {
        if (session != null) return
        generation++
        loginJob?.cancel()
        mutable.value = state.value.copy(busy = false, approval = null, error = null)
    }

    fun signOut() {
        if (!state.value.ready) return
        val service = api
        val epoch = ++generation
        loginJob?.cancel(); refreshJob?.cancel(); pairJob?.cancel(); expiryJob?.cancel()
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
                    error = "Signed out on this device. Secpar could not revoke the session online.")
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
                if (epoch != generation) return@launch
                val removed = withContext(io) {
                    store.owned(saved).filter { access -> computers.none {
                        it.id == access.computerId && it.linked && it.certificateFp == access.pc.fingerprint
                    } }.onEach { store.forgetOwned(saved, it.computerId) }.isNotEmpty()
                }
                if (epoch != generation) return@launch
                mutable.value = state.value.copy(computers = computers, error = null)
                if (removed) changes.value++
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

    private fun message(e: Exception) = (e as? AccountException)?.message ?: "Could not complete the connection. Please try again."
}
