package com.pcremote.home

import com.pcremote.account.FriendComputer
import com.pcremote.account.OwnedComputer
import com.pcremote.agent.AgentState
import com.pcremote.agent.PairedPc
import com.pcremote.nav.Mode

enum class PrimaryAction { OPEN, WAKE, OFFLINE_HELP, CHECKING }

data class PcCardUi(
    val id: String,
    val name: String,
    val online: Boolean,
    val streaming: Boolean,
    val modes: List<Mode>,
    val primary: PrimaryAction,
    /** The host advertises power actions. */
    val canPower: Boolean = false,
    /** Showing its lock or sign-in screen. */
    val locked: Boolean = false,
    /** The host can type a PIN on its lock screen. */
    val canUnlock: Boolean = false,
    val os: String = "",
    /** Certificate fingerprint of the paired computer; ties a saved card to its account entry. */
    val fingerprint: String = "",
)

object HomeModel {
    private fun capModes(caps: Set<String>) = buildList {
        if ("stream" in caps) add(Mode.SCREEN)
        if ("input" in caps) add(Mode.REMOTE)
        if ("media" in caps) add(Mode.MEDIA)
        if ("input" in caps) add(Mode.SLIDES)
        if ("pad" in caps) add(Mode.PAD)
        if ("files" in caps) add(Mode.FILES)
    }

    fun cards(agents: List<PairedPc>, live: Map<String, AgentState>, streaming: Map<String, Boolean>, locked: Map<String, Boolean> = emptyMap()): List<PcCardUi> =
        agents.map { pc ->
            val st = live[pc.id]
            val online = st is AgentState.Online
            val primary = when {
                online -> PrimaryAction.OPEN
                st == null || st is AgentState.Connecting -> PrimaryAction.CHECKING
                pc.macs.isNotEmpty() -> PrimaryAction.WAKE
                else -> PrimaryAction.OFFLINE_HELP
            }
            PcCardUi(pc.id, pc.name, online, streaming[pc.id] == true, (st as? AgentState.Online)?.caps.orEmpty().let(::capModes), primary,
                canPower = st is AgentState.Online && "power" in st.caps,
                locked = online && locked[pc.id] == true,
                canUnlock = st is AgentState.Online && "unlock" in st.caps,
                os = (st as? AgentState.Online)?.pc?.os.orEmpty(),
                fingerprint = pc.fingerprint)
        }
}

/** What a computer card says about the computer. */
enum class ComputerStatus { ONLINE, OFFLINE, STREAMING, LOCKED, CHECKING, HOST_UPDATE_REQUIRED }

/** One entry of the Computers list: a saved computer, or an account computer not saved here yet. */
sealed interface ComputerRow {
    val key: String
    data class Saved(val card: PcCardUi) : ComputerRow { override val key get() = "saved:" + card.id }
    data class FromAccount(val computer: OwnedComputer) : ComputerRow { override val key get() = "account:" + computer.id }
    /** A computer a friend shares. */
    data class Friends(val computer: FriendComputer) : ComputerRow { override val key get() = "friend:" + computer.id }
}

object ComputerList {
    /**
     * Saved computers first, then account computers that are not already saved here. An account
     * computer counts as saved when a card carries the same certificate fingerprint.
     */
    fun rows(cards: List<PcCardUi>, owned: List<OwnedComputer>, shared: List<FriendComputer> = emptyList()): List<ComputerRow> {
        val known = cards.mapNotNull { it.fingerprint.takeIf(String::isNotEmpty) }.toSet()
        return cards.map { ComputerRow.Saved(it) } +
            owned.filter { it.certificateFp == null || it.certificateFp !in known }.map { ComputerRow.FromAccount(it) } +
            shared.filter { s -> cards.none { it.id == s.id } && owned.none { it.id == s.id } }.map { ComputerRow.Friends(it) }
    }

    /** What the owner lets you do on a friend's computer, in the words of the Friends page. */
    fun access(c: FriendComputer): String {
        val control = when {
            c.keyboard && c.mouse -> "Keyboard and mouse"
            c.keyboard -> "Keyboard only"
            c.mouse -> "Mouse only"
            else -> "View only"
        }
        return "${if (c.connect == "always") "Any time" else "Ask first"} · $control"
    }

    fun status(card: PcCardUi): ComputerStatus = when {
        card.locked -> ComputerStatus.LOCKED
        card.streaming -> ComputerStatus.STREAMING
        card.online -> ComputerStatus.ONLINE
        card.primary == PrimaryAction.CHECKING -> ComputerStatus.CHECKING
        else -> ComputerStatus.OFFLINE
    }

    fun status(c: OwnedComputer): ComputerStatus = when {
        !c.linked -> ComputerStatus.HOST_UPDATE_REQUIRED
        c.online -> ComputerStatus.ONLINE
        else -> ComputerStatus.OFFLINE
    }

    fun platform(raw: String): String = when (raw.lowercase()) {
        "windows" -> "Windows"
        "linux" -> "Linux"
        "macos" -> "macOS"
        else -> raw
    }
}
