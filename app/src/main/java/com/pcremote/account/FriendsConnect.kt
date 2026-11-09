package com.pcremote.account

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay

/** Where a request to use a friend's computer stands, for the Friends page. */
enum class GuestPhase { ASKING, WAITING, OPENING }
data class GuestProgress(val computerId: String, val ownerHandle: String, val phase: GuestPhase)

/** What the choices on the Connect panel ask for. View, Audio, Keyboard and mouse, Controller, Clipboard. */
data class GuestChoices(
    val view: Boolean = true, val audio: Boolean = true, val input: Boolean = true,
    val controller: Boolean = true, val clipboard: Boolean = false,
) {
    /** Permission names as the account service takes them. */
    fun permissions(): List<String> = buildList {
        if (view) add("view")
        if (audio) add("audio")
        if (input) { add("keyboard"); add("mouse") }
        if (controller) add("gamepad")
        if (clipboard) add("clipboard")
    }
}

/** Wording for connecting to a friend's computer. */
object FriendsConnect {
    const val HEADING = "Friends' computers"
    const val CONNECT = "Connect"
    const val ASK = "Ask to connect"
    const val VIEW = "View"
    const val AUDIO = "Audio"
    const val INPUT = "Keyboard and mouse"
    const val CONTROLLER = "Controller"
    const val CLIPBOARD = "Clipboard"
    const val OPENING = "Connecting"
    const val CANCEL = "Cancel"

    fun waiting(owner: String) = "Waiting for @$owner"
    fun owner(handle: String) = "@$handle"
    fun phaseText(p: GuestProgress) = if (p.phase == GuestPhase.OPENING) OPENING else waiting(p.ownerHandle)
    fun declined(owner: String) = "@$owner did not allow it."
    fun noAnswer(owner: String) = "@$owner did not answer."
    const val VIEW_ONLY = "@%s lets you view only. Turn off Keyboard and mouse."
    fun viewOnly(owner: String) = VIEW_ONLY.format(owner)
}

sealed interface Approval {
    data object Approved : Approval
    /** `rejected`, `revoked` or any other final state. */
    data class Ended(val state: String) : Approval
    data object Expired : Approval
}

object GuestApproval {
    const val POLL_MS = 2000L
    private val FINAL = setOf("rejected", "revoked", "denied", "claimed", "expired")
    private const val NETWORK_RETRIES = 3

    /**
     * Polls the request every 2 seconds until the owner answers. Stops on a final state, and on the request's
     * own expiry (a Unix second). A few network errors in a row are tolerated; any other error ends the wait.
     */
    suspend fun wait(service: AccountService, first: AccessRequest, now: () -> Long,
        pause: suspend (Long) -> Unit = { delay(it) }): Approval {
        var request = first
        var failures = 0
        while (true) {
            if (request.approved) return Approval.Approved
            if (request.state in FINAL) return Approval.Ended(request.state)
            if (now() >= request.expires) return Approval.Expired
            pause(POLL_MS)
            try {
                request = service.accessStatus(first.id)
                failures = 0
            } catch (e: CancellationException) { throw e }
            catch (e: AccountException) {
                if (e.kind == AccountError.REQUEST_ANSWERED) return Approval.Ended("revoked")
                if (e.kind != AccountError.NETWORK || ++failures > NETWORK_RETRIES) throw e
            }
        }
    }
}
