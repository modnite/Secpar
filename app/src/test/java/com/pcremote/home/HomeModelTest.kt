package com.pcremote.home

import com.pcremote.account.OwnedComputer
import com.pcremote.agent.AgentState
import com.pcremote.agent.FailReason
import com.pcremote.agent.PairedPc
import com.pcremote.agent.PcInfo
import com.pcremote.nav.Mode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class HomeModelTest {
    private fun agent(id: String, macs: List<String> = listOf("AA:BB:CC:DD:EE:FF")) =
        PairedPc(id, "Agent $id", listOf("192.168.0.5"), 47999, "f".repeat(64), "tok", macs)

    private val allCaps = setOf("stream", "input", "media", "pad", "files", "clip", "power", "thumb", "net")
    private fun online(caps: Set<String>) = AgentState.Online(PcInfo("a", "x"), caps)

    @Test fun `online pc gets every mode its caps allow, screen first`() {
        val c = HomeModel.cards(listOf(agent("a")), mapOf("a" to online(allCaps)), mapOf("a" to true)).single()
        assertEquals(listOf(Mode.SCREEN, Mode.REMOTE, Mode.MEDIA, Mode.SLIDES, Mode.PAD, Mode.FILES), c.modes)
        assertTrue(c.online); assertTrue(c.streaming); assertEquals(PrimaryAction.OPEN, c.primary)
    }

    @Test fun `caps limit modes`() {
        assertEquals(listOf(Mode.REMOTE, Mode.SLIDES), HomeModel.cards(listOf(agent("a")), mapOf("a" to online(setOf("input"))), emptyMap()).single().modes)
    }

    @Test fun `offline with mac wakes, without mac shows help, unknown is checking`() {
        val off = mapOf("a" to AgentState.Failed(FailReason.UNREACHABLE))
        assertEquals(PrimaryAction.WAKE, HomeModel.cards(listOf(agent("a")), off, emptyMap()).single().primary)
        assertEquals(PrimaryAction.OFFLINE_HELP, HomeModel.cards(listOf(agent("a", macs = emptyList())), off, emptyMap()).single().primary)
        assertEquals(PrimaryAction.CHECKING, HomeModel.cards(listOf(agent("a")), emptyMap(), emptyMap()).single().primary)
        assertTrue(HomeModel.cards(listOf(agent("a")), off, emptyMap()).single().modes.isEmpty())
    }

    @Test fun `power is offered only when the host advertises it`() {
        assertTrue(HomeModel.cards(listOf(agent("a")), mapOf("a" to online(setOf("power"))), emptyMap()).single().canPower)
        assertFalse(HomeModel.cards(listOf(agent("a")), mapOf("a" to online(setOf("input"))), emptyMap()).single().canPower)
    }

    @Test fun `wake on lan magic packet`() {
        val p = WakeOnLan.packet("AA:BB:CC:DD:EE:FF")!!
        assertEquals(102, p.size)
        assertTrue(p.take(6).all { it == (-1).toByte() })
        assertEquals(0xAA.toByte(), p[6]); assertEquals(0xFF.toByte(), p[101])
        assertNull(WakeOnLan.packet("nope"))
        assertEquals(102, WakeOnLan.packet("aa-bb-cc-dd-ee-ff")!!.size)
    }

    private fun card(id: String, fp: String = "", online: Boolean = true, primary: PrimaryAction = PrimaryAction.OPEN,
        streaming: Boolean = false, locked: Boolean = false) =
        PcCardUi(id, "Card $id", online, streaming, emptyList(), primary, locked = locked, fingerprint = fp)
    private fun owned(id: String, fp: String?, linked: Boolean = true, online: Boolean = true) =
        OwnedComputer(id, "Owned $id", "windows", fp, linked, online)

    @Test fun `card carries the certificate fingerprint`() {
        assertEquals("f".repeat(64), HomeModel.cards(listOf(agent("a")), emptyMap(), emptyMap()).single().fingerprint)
    }

    @Test fun `saved computers come first and account computers already saved are not listed twice`() {
        val rows = ComputerList.rows(listOf(card("a", "fp1"), card("b", "fp2")),
            listOf(owned("x", "fp2"), owned("y", "fp3"), owned("z", null, linked = false)))
        assertEquals(listOf("saved:a", "saved:b", "account:y", "account:z"), rows.map { it.key })
    }

    @Test fun `with nothing saved every account computer is listed`() {
        assertEquals(listOf("account:x"), ComputerList.rows(emptyList(), listOf(owned("x", "fp"))).map { it.key })
        assertTrue(ComputerList.rows(emptyList(), emptyList()).isEmpty())
        val shared = com.pcremote.account.FriendComputer("f", "Studio", "linux", "o", "jamie", "always", keyboard = true, mouse = false)
        assertEquals(listOf("friend:f"), ComputerList.rows(emptyList(), emptyList(), listOf(shared)).map { it.key })
        assertEquals("Any time · Keyboard only", ComputerList.access(shared))
        assertEquals("Ask first · View only", ComputerList.access(shared.copy(connect = "ask", keyboard = false)))
        // A friend's card offers only what the owner's settings allow. Media is never offered. Files only sends.
        assertEquals(listOf(Mode.PAD, Mode.FILES, Mode.SLIDES, Mode.REMOTE).sortedBy { it.ordinal }, ComputerList.friendTiles(shared.copy(keyboard = true, mouse = true)).map { it.mode }.sortedBy { it.ordinal })
        assertEquals(listOf(Mode.PAD, Mode.FILES), ComputerList.friendTiles(shared.copy(keyboard = false, mouse = false)).map { it.mode })
        assertEquals(listOf("mouse"), ComputerList.friendTiles(shared.copy(keyboard = false, mouse = true)).first { it.mode == Mode.REMOTE }.permissions)
    }

    @Test fun `a card without a fingerprint never hides an account computer`() {
        assertEquals(2, ComputerList.rows(listOf(card("a", "")), listOf(owned("x", "fp"))).size)
    }

    @Test fun `status wording union`() {
        assertEquals(ComputerStatus.LOCKED, ComputerList.status(card("a", locked = true, streaming = true)))
        assertEquals(ComputerStatus.STREAMING, ComputerList.status(card("a", streaming = true)))
        assertEquals(ComputerStatus.ONLINE, ComputerList.status(card("a")))
        assertEquals(ComputerStatus.CHECKING, ComputerList.status(card("a", online = false, primary = PrimaryAction.CHECKING)))
        assertEquals(ComputerStatus.OFFLINE, ComputerList.status(card("a", online = false, primary = PrimaryAction.WAKE)))
    }

    @Test fun `account computer status`() {
        assertEquals(ComputerStatus.HOST_UPDATE_REQUIRED, ComputerList.status(owned("x", null, linked = false)))
        assertEquals(ComputerStatus.ONLINE, ComputerList.status(owned("x", "fp")))
        assertEquals(ComputerStatus.OFFLINE, ComputerList.status(owned("x", "fp", online = false)))
    }

    @Test fun `platform labels`() {
        assertEquals("Windows", ComputerList.platform("windows"))
        assertEquals("Linux", ComputerList.platform("linux"))
        assertEquals("macOS", ComputerList.platform("macos"))
        assertEquals("beos", ComputerList.platform("beos"))
    }
}
