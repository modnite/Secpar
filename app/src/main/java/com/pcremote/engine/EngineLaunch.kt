package com.pcremote.engine

import android.content.Context
import android.content.Intent
import com.limelight.Game
import com.limelight.binding.crypto.AndroidCryptoProvider
import com.limelight.nvstream.http.ComputerDetails
import com.limelight.nvstream.http.NvHTTP
import com.limelight.nvstream.http.PairingManager
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.net.NetworkInterface
import java.security.MessageDigest
import java.security.cert.X509Certificate
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket
import javax.net.ssl.X509TrustManager

/**
 * Starts the managed engine that is compiled inside this app: Moonlight's `Game` activity
 * against the tunnel's loopback end (`docs/gamestream-engine.md`). The control connection
 * stays the app's own; the computer decides access on the device it authenticated.
 */
object EngineLaunch {
    /** Whether this APK can run the engine at all. */
    fun present(context: Context): Boolean =
        NativeTunnel.isAvailable() && runCatching { Class.forName(Game::class.java.name, false, context.classLoader) }.isSuccess

    /**
     * Everything up to the stream: ask for the engine, pair this device's Moonlight
     * certificate through the control connection, open the pinned tunnel, and check the
     * engine's own certificate. Returns the intent to start, or a sentence.
     */
    suspend fun prepare(
        context: Context,
        client: GamestreamClient,
        pcName: String,
        settings: com.pcremote.settings.SettingsModel,
        /** The address the control connection is on; used when the computer lists none. */
        fallbackHost: String? = null,
    ): Result<Intent> =
        withContext(Dispatchers.IO) {
            applyStreamSettings(context, settings)
            val endpoint = client.info().getOrElse { return@withContext Result.failure(it) }
            val addresses = endpoint.tunnelAddresses.ifEmpty { listOfNotNull(fallbackHost) }
            val tunnelReady = endpoint.tunnelPort in 1..65535 &&
                endpoint.tunnelCertificate.length == 64 && addresses.isNotEmpty()
            if (!tunnelReady) {
                return@withContext Result.failure(
                    GamestreamProblem("That computer has not opened its stream tunnel yet."),
                )
            }
            val ready = NativeTunnel.begin()
                ?: return@withContext Result.failure(GamestreamProblem("This phone could not open a stream tunnel."))

            val crypto = AndroidCryptoProvider(context)
            val clientCertificate = crypto.clientCertificate.encoded
            val pin = PairingManager.generatePinString()

            client.pair(fingerprint(clientCertificate), pin, ready.certificate, candidates(ready))
                .getOrElse { return@withContext Result.failure(it) }

            NativeTunnel.open(serdeJsonList(addresses), endpoint.tunnelPort, endpoint.tunnelCertificate)?.let { said ->
                NativeTunnel.close()
                // The tunnel library reports a sentence; short codes keep their own words.
                return@withContext Result.failure(GamestreamProblem(if (' ' in said) said else problem(said)))
            }

            try {
                start(context, endpoint, crypto, pin, pcName)
            } catch (e: Exception) {
                NativeTunnel.close()
                Result.failure(GamestreamProblem(problem(e)))
            }
        }

    /** The stream component runs; nothing here keeps the tunnel beyond that. */
    fun finish() {
        streaming = false
        NativeTunnel.close()
    }

    /** Set while the engine's own activity is on screen. */
    @Volatile
    private var streaming = false

    /** Starts the stream and remembers to close the tunnel when the app is looked at again. */
    fun launch(context: Context, intent: Intent) {
        streaming = true
        context.startActivity(intent)
    }

    /** Called when the app comes back to the foreground: the stream is over. */
    fun finishIfStreaming() {
        if (streaming) finish()
    }

    private fun start(
        context: Context,
        endpoint: EngineEndpoint,
        crypto: AndroidCryptoProvider,
        pin: String,
        pcName: String,
    ): Result<Intent> {
        // The tunnel forwards the engine's own ports, so they keep their numbers on loopback.
        val server = serverCertificate(Gamestream.LOOPBACK, endpoint.httpsPort)
            ?: return Result.failure(GamestreamProblem("The stream engine did not answer on this computer."))
        if (fingerprint(server.encoded) != endpoint.certificate) {
            return Result.failure(
                GamestreamProblem("The stream engine's certificate does not match what the computer reported."),
            )
        }

        val where = ComputerDetails.AddressTuple(Gamestream.LOOPBACK, endpoint.httpPort)
        var http = NvHTTP(where, endpoint.httpsPort, Gamestream.MOONLIGHT_CLIENT_ID, server, crypto)
        val info = http.getServerInfo(true)
        val details = http.getComputerDetails(info)

        // The computer approved this certificate during `gamestream.pair`. If the engine
        // still asks for the handshake, it completes with the same PIN, without a prompt.
        if (http.getPairState(info) != PairingManager.PairState.PAIRED) {
            val pairing = PairingManager(http, crypto)
            val state = pairing.pair(info, pin)
            val paired = pairing.pairedCert
            if (state != PairingManager.PairState.PAIRED || paired == null) {
                return Result.failure(GamestreamProblem("The stream engine refused this device."))
            }
            http = NvHTTP(where, endpoint.httpsPort, Gamestream.MOONLIGHT_CLIENT_ID, paired, crypto)
        }
        val desktop = http.getAppByName("Desktop")
            ?: return Result.failure(GamestreamProblem("That computer has no desktop to stream."))

        return Result.success(
            Intent(context, Game::class.java).apply {
                putExtra(Game.EXTRA_HOST, Gamestream.LOOPBACK)
                putExtra(Game.EXTRA_PORT, endpoint.httpPort)
                putExtra(Game.EXTRA_HTTPS_PORT, endpoint.httpsPort)
                putExtra(Game.EXTRA_APP_NAME, desktop.appName)
                putExtra(Game.EXTRA_APP_ID, desktop.appId)
                putExtra(Game.EXTRA_APP_HDR, desktop.isHdrSupported)
                putExtra(Game.EXTRA_UNIQUEID, Gamestream.MOONLIGHT_CLIENT_ID)
                putExtra(Game.EXTRA_PC_UUID, details.uuid ?: "")
                putExtra(Game.EXTRA_PC_NAME, pcName)
                putExtra(Game.EXTRA_SERVER_CERT, server.encoded)
            },
        )
    }

    /**
     * Moonlight reads its own settings. They are written from the app's choices before the
     * stream starts, so resolution, frame rate, bitrate and audio come from Secpar's settings
     * instead of Moonlight's untouched defaults.
     */
    private fun applyStreamSettings(context: Context, settings: com.pcremote.settings.SettingsModel) {
        val prefs = android.preference.PreferenceManager.getDefaultSharedPreferences(context)
        prefs.edit()
            .putString("list_resolution", "${settings.resolution.first}x${settings.resolution.second}")
            .putString("list_fps", settings.frameRate.toString())
            .putInt("seekbar_bitrate_kbps", settings.bitrateMbps * 1000)
            .putBoolean("checkbox_host_audio", !settings.playHostAudio)
            // Leaving the stream for another app keeps it in a small window, like the app's own.
            .putBoolean("checkbox_enable_pip", true)
            .apply()
    }

    /** The tunnel socket's own endpoints: what is seen from outside first, then the LAN. */
    private fun candidates(ready: NativeTunnel.Ready): List<String> {
        val found = ArrayList<String>()
        ready.seen?.let { found += it }
        for (address in lanAddresses()) found += "$address:${ready.port}"
        return Gamestream.candidates(found)
    }

    private fun lanAddresses(): List<String> = try {
        NetworkInterface.getNetworkInterfaces().toList()
            .filter { it.isUp && !it.isLoopback }
            .flatMap { it.inetAddresses.toList() }
            .filterIsInstance<java.net.Inet4Address>()
            .map { it.hostAddress }
    } catch (e: Exception) {
        emptyList()
    }

    /** The engine's own certificate, read from its TLS handshake. The pin is checked after. */
    private fun serverCertificate(host: String, port: Int): X509Certificate? {
        val trustAll = object : X509TrustManager {
            override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) {}
            override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {}
            override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
        }
        return try {
            val context = SSLContext.getInstance("TLS")
            context.init(null, arrayOf(trustAll), null)
            (context.socketFactory.createSocket(host, port) as SSLSocket).apply {
                soTimeout = 5_000
                startHandshake()
            }.use { socket ->
                socket.session.peerCertificates.firstOrNull() as? X509Certificate
            }
        } catch (e: Exception) {
            null
        }
    }

    private fun fingerprint(certificate: ByteArray): String =
        MessageDigest.getInstance("SHA-256").digest(certificate).joinToString("") { "%02x".format(it) }

    /** The host expects a JSON array of IPs, as `gamestream.info` sends them. */
    private fun serdeJsonList(values: List<String>): String =
        values.joinToString(prefix = "[", postfix = "]") { "\"$it\"" }

    private fun problem(code: String): String = when (code) {
        "not_reachable" -> "That computer did not accept the stream tunnel."
        "bad_pin" -> "That computer sent a stream certificate that cannot be used."
        "no_addresses" -> "That computer did not say where its stream listens."
        "no_socket" -> "This phone could not open the stream tunnel."
        else -> "The stream tunnel did not start."
    }

    private fun problem(error: Exception): String =
        error.message?.takeIf { it.isNotBlank() } ?: "The stream did not start."
}
