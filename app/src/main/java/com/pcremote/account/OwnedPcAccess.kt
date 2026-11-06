package com.pcremote.account

import com.pcremote.agent.AccountPcAccess
import com.pcremote.agent.PairedPc

/** Account overlay for existing Home/mode code; never copies credentials to the manual store. */
class OwnedPcAccess(private val store: AccountStore, private val currentOwner: (() -> String?)? = null) : AccountPcAccess {
    private fun session(): AccountSession? = store.session()?.takeIf {
        currentOwner == null || currentOwner.invoke() == it.user.id
    }

    override fun all(): List<PairedPc> {
        val session = session() ?: return emptyList()
        return store.owned(session).map { it.pc.copy(accountOwner = session.user.id, accountComputer = it.computerId) }
    }

    override fun upsert(pc: PairedPc) {
        val session = session() ?: return
        if (session.user.id != pc.accountOwner) return
        val current = store.owned(session).firstOrNull { it.computerId == pc.accountComputer } ?: return
        // A stale LAN/relay callback from an older session cannot replace a newly issued credential.
        if (current.pc.id != pc.id || current.pc.token != pc.token || current.pc.fingerprint != pc.fingerprint) return
        store.saveOwned(session, OwnedAccess(current.computerId, pc))
    }

    override fun remove(id: String) {
        val session = session() ?: return
        store.owned(session).firstOrNull { it.pc.id == id }?.let { store.forgetOwned(session, it.computerId) }
    }

    companion object {
        const val ROUTE_PREFIX = "owned:"
        fun routeFor(computerId: String) = ROUTE_PREFIX + computerId
    }
}
