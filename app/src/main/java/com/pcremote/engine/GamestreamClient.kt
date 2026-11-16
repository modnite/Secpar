package com.pcremote.engine

import com.pcremote.agent.Msg
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withTimeoutOrNull
import java.util.concurrent.atomic.AtomicLong

/**
 * The engine exchange on the connection the app already has open (`docs/gamestream-engine.md`).
 * One request, one answer, matched by id; a request that is not answered says so instead of
 * waiting forever. Only the computer decides who may pair, and only on the device it
 * authenticated, so nothing here is a permission check.
 */
class GamestreamClient(
    private val send: (Msg) -> Boolean,
    private val incoming: Flow<Msg>,
    private val timeoutMs: Long = 6_000,
    /** The computer answers a pairing after the engine has accepted it, which takes longer. */
    /** Longer than the computer waits for this device's pairing handshake. */
    private val pairTimeoutMs: Long = 45_000,
) {
    private val counter = AtomicLong()

    private sealed interface Answer {
        data class Reply(val message: Msg) : Answer
        data object Closed : Answer
        data object Quiet : Answer
    }

    private fun nextId(): String = "gs${counter.incrementAndGet()}"

    /** The engine's ports and pins, or a sentence saying why they are not usable. */
    suspend fun info(): Result<EngineEndpoint> {
        val id = nextId()
        return when (val answer = ask(id) { Msg.GameStreamInfoAsk(id) }) {
            is Answer.Reply -> when (val reply = answer.message) {
                is Msg.GameStreamInfo ->
                    if (reply.id == id) Gamestream.read(reply)
                    else Result.failure(GamestreamProblem(OTHER_REQUEST))
                is Msg.GameStreamError -> Result.failure(GamestreamProblem(Gamestream.problem(reply.code, reply.msg)))
                else -> Result.failure(GamestreamProblem(QUIET))
            }
            Answer.Closed -> Result.failure(GamestreamProblem(CLOSED))
            Answer.Quiet -> Result.failure(GamestreamProblem(QUIET))
        }
    }

    /**
     * Asks the computer to accept this device's engine certificate. [candidates] are the tunnel
     * socket's own UDP endpoints so the computer can open the way towards this device.
     */
    suspend fun pair(
        clientCertificate: String,
        pin: String,
        tunnelCertificate: String,
        candidates: List<String>,
    ): Result<Unit> {
        val id = nextId()
        val request = Gamestream.pairRequest(id, clientCertificate, pin, tunnelCertificate, candidates)
            .getOrElse { return Result.failure(it) }
        return when (val answer = ask(id, pairTimeoutMs) { request }) {
            is Answer.Reply -> when (val reply = answer.message) {
                is Msg.GameStreamPaired ->
                    if (reply.id == id) Result.success(Unit)
                    else Result.failure(GamestreamProblem(OTHER_REQUEST))
                is Msg.GameStreamError -> Result.failure(GamestreamProblem(Gamestream.problem(reply.code, reply.msg)))
                else -> Result.failure(GamestreamProblem(QUIET))
            }
            Answer.Closed -> Result.failure(GamestreamProblem(CLOSED))
            Answer.Quiet -> Result.failure(GamestreamProblem(QUIET))
        }
    }

    private suspend fun ask(id: String, waitMs: Long = timeoutMs, build: () -> Msg): Answer = coroutineScope {
        // Subscribe before sending: a bounded shared flow drops anything emitted first.
        val answer = async(start = CoroutineStart.UNDISPATCHED) { await(id, waitMs) }
        if (!send(build())) {
            answer.cancel()
            return@coroutineScope Answer.Closed
        }
        answer.await()?.let(Answer::Reply) ?: Answer.Quiet
    }

    /** The first answer to [id], or null when the connection stays quiet. */
    private suspend fun await(id: String, waitMs: Long): Msg? = withTimeoutOrNull(waitMs) {
        incoming.first { m ->
            when (m) {
                is Msg.GameStreamInfo -> m.id == id
                is Msg.GameStreamPaired -> m.id == id
                is Msg.GameStreamError -> m.id == id
                else -> false
            }
        }
    }

    private companion object {
        const val QUIET = "That computer did not answer."
        const val CLOSED = "The connection to that computer is closed."
        const val OTHER_REQUEST = "That computer sent an answer for another request."
    }
}
