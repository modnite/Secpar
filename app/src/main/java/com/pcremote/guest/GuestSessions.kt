package com.pcremote.guest

import com.pcremote.account.GuestGrant
import com.pcremote.agent.AgentConnection
import com.pcremote.agent.AgentState
import com.pcremote.agent.PairedPc
import com.pcremote.agent.RELAY_HOST
import com.pcremote.agent.RelayWs
import com.pcremote.agent.Ws
import com.pcremote.agent.WsFactory
import com.pcremote.agent.WsListener
import kotlinx.coroutines.CoroutineScope
import java.util.concurrent.ConcurrentHashMap

/** The socket to a friend's computer, joined and waiting for the stream screen to take it over. */
class GuestLink(val route: String, val pc: PairedPc, val conn: AgentConnection, val ownerHandle: String, val permissions: Set<String>)

/** Joined friend sessions by route. A route starts with [PREFIX], so it never matches a paired computer. */
object GuestSessions {
    const val PREFIX = "guest:"
    private val links = ConcurrentHashMap<String, GuestLink>()

    fun isGuest(route: String) = route.startsWith(PREFIX)
    fun add(link: GuestLink) { links[link.route] = link }
    fun find(route: String): GuestLink? = links[route]
    fun remove(route: String) { links.remove(route) }
    fun allows(route: String, permission: String) = links[route]?.permissions?.contains(permission) == true
}

/** Opens the relay socket with normal certificate checking. The host, port and fingerprint of a paired PC do not apply. */
class GuestWs(private val url: String) : WsFactory {
    override fun open(host: String, port: Int, fingerprint: String, listener: WsListener): Ws = RelayWs.openUrl(url, listener)
}

class GuestException(message: String) : Exception(message)

object GuestConnector {
    /**
     * Opens `/v1/guest?pc=...`, sends the grant as the first frame and waits for `welcome`. Returns the
     * route of the joined session, which the stream screen opens like a paired computer. Throws
     * [GuestException] with a plain message when the relay or the host refuses.
     */
    suspend fun open(scope: CoroutineScope, grant: GuestGrant, computerName: String, ownerHandle: String,
        permissions: Set<String>, routeId: String, factory: (String) -> WsFactory = ::GuestWs): String {
        val url = GuestRelay.socketUrl(grant.relay, grant.pc) ?: throw GuestException("Unexpected answer from Secpar.")
        val conn = AgentConnection(listOf(RELAY_HOST), 0, "", scope, factory(url))
        try { conn.guest(grant.grant) } catch (e: Throwable) { conn.close(); throw e }
        val online = conn.state.value as? AgentState.Online
        if (online == null) {
            val code = conn.lastError
            val text = if (code != null) GuestMessages.forCode(code, conn.lastErrorMsg) else "Could not reach the computer."
            conn.close()
            throw GuestException(text)
        }
        val route = GuestSessions.PREFIX + routeId
        val pc = PairedPc(route, computerName.ifBlank { online.pc.name }, emptyList(), 0, "", "", emptyList())
        GuestSessions.add(GuestLink(route, pc, conn, ownerHandle, permissions))
        return route
    }
}
