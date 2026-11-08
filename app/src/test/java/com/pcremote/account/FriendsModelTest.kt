package com.pcremote.account

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class FriendsModelTest {
    private val me = "me"
    private fun friend(state: String, requester: String) = Friend("f", "river", state, requester)

    @Test fun `row label follows the state and who asked`() {
        assertEquals(FriendLabel.FRIEND, FriendsModel.label(friend("accepted", me), me))
        assertEquals(FriendLabel.ACCEPT, FriendsModel.label(friend("pending", "them"), me))
        assertEquals(FriendLabel.REQUEST_SENT, FriendsModel.label(friend("pending", me), me))
    }

    @Test fun `label text is the desktop wording`() {
        assertEquals("Friend", FriendsModel.labelText(FriendLabel.FRIEND))
        assertEquals("Accept", FriendsModel.labelText(FriendLabel.ACCEPT))
        assertEquals("Request sent", FriendsModel.labelText(FriendLabel.REQUEST_SENT))
    }

    @Test fun `menu offers cancel for a pending request and remove for a friend`() {
        assertEquals("Cancel request", FriendsModel.removeText(friend("pending", me)))
        assertEquals("Remove friend", FriendsModel.removeText(friend("accepted", me)))
    }

    @Test fun `avatar shows the first letter in capitals`() {
        assertEquals("R", FriendsModel.initial("river"))
        assertEquals("?", FriendsModel.initial(""))
    }

    @Test fun `add needs a code and no change in flight`() {
        assertFalse(FriendsModel.canAdd("", false))
        assertFalse(FriendsModel.canAdd("   ", false))
        assertFalse(FriendsModel.canAdd("code", true))
        assertTrue(FriendsModel.canAdd("code", false))
    }

    @Test fun `friend codes are cleaned and checked`() {
        val code = "123e4567-e89b-42d3-a456-426614174000"
        assertEquals(code, AccountText.cleanCode(" " + code.uppercase() + "\n"))
        assertNull(AccountText.cleanCode("../me"))
        assertNull(AccountText.cleanCode(code + "0"))
        assertNull(AccountText.cleanCode(""))
    }

    @Test fun `page strings match the desktop client`() {
        assertEquals("No friends yet", FriendsModel.EMPTY)
        assertEquals("Sign in to add friends.", FriendsModel.SIGN_IN_PROMPT)
        assertEquals("Paste a friend code", FriendsModel.CODE_HINT)
        assertEquals("Blocked people", FriendsModel.BLOCKED_HEADING)
        assertEquals("Confirm this code in your browser", FriendsModel.CONFIRM_IN_BROWSER)
        assertEquals("Signed out here. Could not revoke the remote session while offline.", AccountText.SIGNED_OUT_OFFLINE)
    }
}
