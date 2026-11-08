package com.pcremote.account

/** What the Friends page says about one friend. Wording matches desktop/src/account.rs. */
enum class FriendLabel { FRIEND, ACCEPT, REQUEST_SENT }

object FriendsModel {
    const val SIGN_IN_PROMPT = "Sign in to add friends."
    const val CREATE_ACCOUNT = "Create an account"
    const val CREATE_ACCOUNT_URL = "https://accounts.secpar.win/auth/register"
    const val USERNAME_HINT = "Add by username"
    const val ADD = "Add"
    const val EMPTY = "No friends yet"
    const val BLOCKED_HEADING = "Blocked people"
    const val UNBLOCK = "Unblock"
    const val BLOCK = "Block"
    const val REMOVE = "Remove friend"
    const val CANCEL_REQUEST = "Cancel request"
    const val ACCEPT = "Accept"
    const val CONFIRM_IN_BROWSER = "Confirm this code in your browser"

    /** The state shown at the right of a row. */
    fun label(friend: Friend, ownId: String): FriendLabel = when {
        friend.accepted -> FriendLabel.FRIEND
        friend.pending && friend.requester != ownId -> FriendLabel.ACCEPT
        else -> FriendLabel.REQUEST_SENT
    }

    fun labelText(label: FriendLabel) = when (label) {
        FriendLabel.FRIEND -> "Friend"
        FriendLabel.ACCEPT -> ACCEPT
        FriendLabel.REQUEST_SENT -> "Request sent"
    }

    /** The second item of the row menu. */
    fun removeText(friend: Friend) = if (friend.pending) CANCEL_REQUEST else REMOVE

    /** The letter in the avatar. */
    fun initial(handle: String): String = handle.firstOrNull { !it.isWhitespace() }?.uppercase() ?: "?"

    /** Tells people which name to give their friends. */
    fun youAre(handle: String) = "You are @$handle"

    /** Add stays off until there is something to add. */
    fun canAdd(username: String, working: Boolean) = !working && username.isNotBlank()
}
