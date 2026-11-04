package com.pcremote.agent

import java.net.URLDecoder

/** The pairing QR payload: `pcremote://pair?n=&h=&p=&fp=&t=` (docs/protocol.md). */
data class PairUri(val name: String, val hosts: List<String>, val port: Int, val fingerprint: String, val otp: String, val relayUrl: String? = null, val pcId: String? = null) {
    companion object {
        private val PREFIXES = listOf("secpar://pair?", "pcremote://pair?")
        private val HEX64 = Regex("^[0-9a-f]{64}$")

        fun parse(s: String): PairUri? {
            val prefix = PREFIXES.firstOrNull { s.startsWith(it) } ?: return null
            val q = s.removePrefix(prefix).split('&').mapNotNull {
                val i = it.indexOf('=')
                if (i <= 0) null else it.substring(0, i) to runCatching { URLDecoder.decode(it.substring(i + 1), "UTF-8") }.getOrElse { return null }
            }.toMap()
            val relay = q["r"]
            val pc = q["pc"]
            if (relay != null) {
                val u = runCatching { java.net.URI(relay) }.getOrNull() ?: return null
                if (u.scheme != "wss" || u.host.isNullOrEmpty() || u.userInfo != null || u.fragment != null ||
                    u.path != "/v1/phone" || pc.isNullOrBlank()) return null
            }
            val hosts = q["h"].orEmpty().split(',').map { it.trim() }.filter { it.isNotEmpty() }
            val port = q["p"]?.toIntOrNull() ?: return null
            val fp = q["fp"]?.lowercase() ?: return null
            val otp = q["t"].orEmpty()
            if ((hosts.isEmpty() && relay == null) || port !in 1..65535 || !HEX64.matches(fp) || otp.isEmpty()) return null
            return PairUri(q["n"].orEmpty().ifEmpty { "PC" }, hosts, port, fp, otp, relay, pc)
        }
    }
}
