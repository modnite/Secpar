package com.pcremote.account

/** What the Computers and Friends tabs can ask of the account. Defaults do nothing, for fixtures. */
class AccountActions(
    /** Email, password, and a callback that gets null on success or the words to show. */
    val signIn: (String, String, (String?) -> Unit) -> Unit = { _, _, _ -> },
    /** register, resend, verify, forgot or reset, with its fields. */
    val auth: (String, Map<String, String>, (String?) -> Unit) -> Unit = { _, _, _ -> },
    val cancelSignIn: () -> Unit = {},
    val signOut: () -> Unit = {},
    val refresh: () -> Unit = {},
    val connect: (OwnedComputer) -> Unit = {},
    val cancelConnect: () -> Unit = {},
    val dismissError: () -> Unit = {},
    val friend: (FriendAction, String) -> Unit = { _, _ -> },
    val note: (String) -> Unit = {},
    val search: (String) -> Unit = {},
    val setVisible: (Boolean) -> Unit = {},
    val friendSetting: (String, String?, String?) -> Unit = { _, _, _ -> },
    val friendInput: (String, Boolean?, Boolean?) -> Unit = { _, _, _ -> },
    val answer: (Answer, String) -> Unit = { _, _ -> },
    /** Ask to use a friend's computer with these permissions. */
    val connectFriend: (FriendComputer, List<String>, com.pcremote.nav.Mode) -> Unit = { _, _, _ -> },
    val cancelFriendConnect: () -> Unit = {},
    /** Turn request notifications on or off. */
    val setAlerts: (Boolean) -> Unit = {},
)
