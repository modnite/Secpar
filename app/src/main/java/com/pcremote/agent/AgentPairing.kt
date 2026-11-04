package com.pcremote.agent

import com.pcremote.onboarding.AgentPairException
import com.pcremote.onboarding.AgentPaired
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first

/** Adapts [AgentConnection.pair] to the onboarding model. The connection is closed afterwards. */
suspend fun agentPair(scope: CoroutineScope, uri: PairUri, device: String, pin: String): Result<AgentPaired> {
    val factory = WsFactory { host, port, fp, listener ->
        if (host == RELAY_HOST && uri.relayUrl != null) RelayWs.openUrl(uri.relayUrl, listener)
        else OkHttpWs.open(host, port, fp, listener)
    }
    // Fresh remote-capable QR codes try the relay first; old QR codes keep the direct path.
    val hosts = listOfNotNull(uri.relayUrl?.let { RELAY_HOST }) + uri.hosts
    val c = AgentConnection(hosts, uri.port, uri.fingerprint, scope, factory)
    try {
        val token = c.pair(uri.otp, device, pin).getOrElse {
            val reason = (c.state.value as? AgentState.Failed)?.reason ?: FailReason.UNREACHABLE
            return Result.failure(AgentPairException(reason))
        }
        val pc = (c.state.value as? AgentState.Online)?.pc ?: PcInfo(uri.name, uri.name)
        if (uri.pcId != null && uri.pcId != pc.id)
            return Result.failure(AgentPairException(FailReason.CERT_MISMATCH))
        val remote = withTimeoutOrNull(2_000) { c.remote.filterNotNull().first() }
        val relay = remote?.let { m -> RelayInfo.b64url(m.ticket)?.takeIf { it.size == 32 }?.let { RelayInfo(m.url, pc.id, m.dev, it) } }
        if (c.connectedHost == RELAY_HOST && relay == null)
            return Result.failure(AgentPairException(FailReason.UNREACHABLE))
        return Result.success(AgentPaired(token, pc, c.connectedHost ?: uri.hosts.firstOrNull().orEmpty(), relay))
    } finally {
        c.close()
    }
}
