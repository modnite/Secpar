package com.pcremote.engine

import com.pcremote.agent.Msg
import java.net.InetAddress

/** A refusal that already reads as a sentence for the stream menu. */
class GamestreamProblem(message: String) : Exception(message)

/**
 * Where the managed engine answers, as `docs/gamestream-engine.md` describes it on the
 * existing authenticated connection. The tunnel fields stay absent until the computer has
 * opened the engine to this device; [tunnelReady] says whether a tunnel can be opened.
 */
data class EngineEndpoint(
    val httpPort: Int,
    val httpsPort: Int,
    /** The engine's client-certificate pin, lowercase hex. */
    val certificate: String,
    /** The computer's tunnel endpoint; 0, empty and no addresses until it advertises one. */
    val tunnelPort: Int = 0,
    val tunnelCertificate: String = "",
    val tunnelAddresses: List<String> = emptyList(),
) {
    val tunnelReady: Boolean
        get() = tunnelPort in 1..65535 && tunnelCertificate.length == 64 && tunnelAddresses.isNotEmpty()
}

/**
 * The engine's half of the protocol, without sockets. The host answers `gamestream.info` with
 * the engine ports and certificate pin; `gamestream.pair` accepts this device's engine
 * certificate and tunnel identity. Nothing here decides access: the computer does that on the
 * device it authenticated.
 */
object Gamestream {
    const val CAPABILITY = "gamestream"

    /** Moonlight listens to the tunnel on the phone's loopback; nothing else uses this port. */
    const val LOOPBACK = "127.0.0.1"
    const val ENGINE_BASE = 46989

    /**
     * Moonlight's connection code presents this one id to every engine whatever id it is
     * given, so the computer must be told the same one (`docs/dock-input.md` history and the
     * working android-secpar client). What tells devices apart is the certificate.
     */
    const val MOONLIGHT_CLIENT_ID = "0123456789ABCDEF"

    fun offers(caps: Set<String>): Boolean = CAPABILITY in caps

    /** A lowercase hex SHA-256 pin, or null. */
    fun fingerprint(value: String): String? {
        val pin = value.trim().lowercase()
        return if (pin.length == 64 && pin.all { it in '0'..'9' || it in 'a'..'f' }) pin else null
    }

    /** An IP literal as `gamestream.info` carries them. Names are refused; nothing resolves here. */
    fun address(value: String): String? {
        val text = value.trim().removeSurrounding("[", "]")
        if (text.isEmpty() || !text.any { it == '.' || it == ':' }) return null
        if (!text.all { it.isDigit() || it in 'a'..'f' || it in 'A'..'F' || it == '.' || it == ':' || it == '%' }) return null
        return try {
            InetAddress.getByName(text).hostAddress
        } catch (e: Exception) {
            null
        }
    }

    /** `ip:port` as the computer receives it for punching; null when it is not one. */
    fun candidate(value: String): String? {
        val text = value.trim()
        val bracketed = text.startsWith("[")
        val split = if (bracketed) text.indexOf("]:") else text.lastIndexOf(':')
        if (split <= 0) return null
        val host = if (bracketed) text.substring(1, split) else text.substring(0, split)
        val port = text.substring(if (bracketed) split + 2 else split + 1).toIntOrNull() ?: return null
        val ip = address(host) ?: return null
        if (port !in 1..65535) return null
        return if (':' in ip) "[$ip]:$port" else "$ip:$port"
    }

    fun candidates(values: List<String>): List<String> = values.mapNotNull(::candidate).distinct()

    /** The host's answer as an endpoint, or a sentence saying what is wrong with it. */
    fun read(reply: Msg.GameStreamInfo): Result<EngineEndpoint> {
        val certificate = fingerprint(reply.certificate_sha256)
            ?: return Result.failure(GamestreamProblem("That computer did not send a valid stream certificate."))
        if (reply.http_port !in 1..65535 || reply.https_port !in 1..65535) {
            return Result.failure(GamestreamProblem("That computer sent stream ports that cannot be used."))
        }
        val tunnelCertificate = fingerprint(reply.tunnel_certificate_sha256)
        val addresses = reply.tunnel_addresses.mapNotNull(::address)
        if (reply.tunnel_port != 0 || reply.tunnel_certificate_sha256.isNotEmpty() || reply.tunnel_addresses.isNotEmpty()) {
            // A half-filled tunnel would make the phone dial the wrong place; refuse it whole.
            if (reply.tunnel_port !in 1..65535 || tunnelCertificate == null || addresses.isEmpty()) {
                return Result.failure(GamestreamProblem("That computer sent an incomplete stream tunnel."))
            }
        }
        return Result.success(
            EngineEndpoint(
                httpPort = reply.http_port,
                httpsPort = reply.https_port,
                certificate = certificate,
                tunnelPort = reply.tunnel_port,
                tunnelCertificate = tunnelCertificate.orEmpty(),
                tunnelAddresses = addresses,
            ),
        )
    }

    /** The pair request, with the tunnel identity the computer allow-lists for this device. */
    fun pairRequest(
        id: String,
        clientCertificate: String,
        pin: String,
        tunnelCertificate: String,
        candidates: List<String>,
    ): Result<Msg.GameStreamPair> {
        val certificate = fingerprint(clientCertificate)
            ?: return Result.failure(GamestreamProblem("This device has no usable stream certificate."))
        if (pin.length != MOONLIGHT_PIN_LENGTH || !pin.all { it.isDigit() }) {
            return Result.failure(GamestreamProblem("The pairing PIN must be four digits."))
        }
        val tunnel = fingerprint(tunnelCertificate)
            ?: return Result.failure(GamestreamProblem("This device has no usable tunnel certificate."))
        val idOk = id.length in 1..64 && id.all { it in 'a'..'z' || it in 'A'..'Z' || it.isDigit() || it == '_' || it == '-' }
        if (!idOk) return Result.failure(GamestreamProblem("The pairing request has no usable id."))
        return Result.success(
            Msg.GameStreamPair(id, MOONLIGHT_CLIENT_ID, certificate, pin, tunnel, candidates(candidates)),
        )
    }

    /** The host's own words when they are plain, otherwise Secpar's. */
    fun problem(code: String, message: String = ""): String {
        val said = message.trim()
        if (said.isNotEmpty() && said != "null") return said
        return when (code) {
            "unavailable" -> "That computer cannot start a stream right now."
            "bad_request" -> "That computer did not accept the request."
            "busy" -> "That computer is pairing another device."
            "pair_failed" -> "That computer did not accept this device."
            "unsupported" -> "That computer has no stream engine for this device."
            "unauthorized" -> "That computer no longer trusts this device. Connect it again."
            else -> "That computer did not answer."
        }
    }

    private const val MOONLIGHT_PIN_LENGTH = 4
}
