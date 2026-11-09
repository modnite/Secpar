package com.pcremote.account

/** What the Computers and Friends tabs can ask of the account. Defaults do nothing, for fixtures. */
class AccountActions(
    val signIn: () -> Unit = {},
    val cancelSignIn: () -> Unit = {},
    val signOut: () -> Unit = {},
    val refresh: () -> Unit = {},
    val openBrowser: () -> Unit = {},
    val connect: (OwnedComputer) -> Unit = {},
    val cancelConnect: () -> Unit = {},
    val dismissError: () -> Unit = {},
    val friend: (FriendAction, String) -> Unit = { _, _ -> },
    val note: (String) -> Unit = {},
    val search: (String) -> Unit = {},
    val setVisible: (Boolean) -> Unit = {},
    val friendSetting: (String, String?, String?) -> Unit = { _, _, _ -> },
    val answer: (Answer, String) -> Unit = { _, _ -> },
    /** Ask to use a friend's computer with these permissions. */
    val connectFriend: (FriendComputer, List<String>) -> Unit = { _, _ -> },
    val cancelFriendConnect: () -> Unit = {},
    /** Turn request notifications on or off. */
    val setAlerts: (Boolean) -> Unit = {},
)
