package com.pcremote.guest

/** Checks what the account service says about a friend's computer before anything is opened. */
object GuestRelay {
    private val PC_ID = Regex("[A-Za-z0-9_-]{1,64}")
    private val HOST = Regex("[A-Za-z0-9]([A-Za-z0-9.-]{0,251}[A-Za-z0-9])?")

    fun validPc(pc: String) = PC_ID.matches(pc)

    /**
     * `wss://<relay host>/v1/guest?pc=<pc>` for an https origin, or null when the origin has anything
     * besides scheme, host and port, or the pc id is not plain. Nothing from the answer can add a
     * path, query, user name or second host.
     */
    fun socketUrl(relay: String, pc: String): String? {
        if (!validPc(pc) || relay.length > 300 || relay.any { it <= ' ' || it == '\\' || it == '\u007f' }) return null
        val uri = try { java.net.URI(relay) } catch (_: Exception) { return null }
        val host = uri.host ?: return null
        val port = uri.port
        val ok = uri.scheme == "https" && uri.rawUserInfo == null && uri.rawQuery == null && uri.rawFragment == null &&
            (uri.rawPath.isNullOrEmpty() || uri.rawPath == "/") && HOST.matches(host) && (port == -1 || port in 1..65535) &&
            uri.rawAuthority == (if (port == -1) host else "$host:$port")
        if (!ok) return null
        return "wss://$host${if (port == -1) "" else ":$port"}/v1/guest?pc=$pc"
    }
}

/** Plain words for the codes a host sends when it refuses or ends a friend's session. */
object GuestMessages {
    fun forCode(code: String?, reason: String = ""): String = when (code) {
        "guest_refused" -> "The computer did not accept the request. Ask again."
        "busy" -> "The computer is busy. Try again later."
        "locked" -> "The computer is locked."
        "forbidden" -> "The owner does not allow that."
        "guest_ended" -> reason.trim().takeIf { it.isNotEmpty() && it.length <= 120 && it.none { c -> c < ' ' } }
            ?.let { "The owner ended the session: $it" } ?: "The owner ended the session."
        "pad_limit" -> "The computer has no free controller slot."
        else -> "The computer refused the connection."
    }

    /** Codes after which the session is over. `forbidden` and `pad_limit` leave it open. */
    fun ends(code: String?) = code == "guest_ended" || code == "guest_refused"
}
