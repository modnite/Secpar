package com.pcremote.account

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import java.util.concurrent.TimeUnit

/**
 * The account's live channel. The service sends "changed" whenever something this account sees
 * changes, and [onChanged] fetches the lists again. Polling stays as the fallback.
 */
object AccountEvents {
    private const val URL = "wss://accounts.secpar.win/v1/events"
    private val client by lazy {
        OkHttpClient.Builder().followRedirects(false).readTimeout(0, TimeUnit.MILLISECONDS).build()
    }

    fun start(scope: CoroutineScope, token: String, onChanged: () -> Unit): Job = scope.launch(Dispatchers.IO) {
        var wait = 1_000L
        while (isActive) {
            val opened = System.currentTimeMillis()
            val closed = CompletableDeferred<Unit>()
            val socket = client.newWebSocket(Request.Builder().url(URL).header("Authorization", "Bearer $token").build(),
                object : WebSocketListener() {
                    // Anything missed while the channel was down.
                    override fun onOpen(webSocket: WebSocket, response: Response) = onChanged()
                    override fun onMessage(webSocket: WebSocket, text: String) { if (text == "changed") onChanged() }
                    override fun onClosing(webSocket: WebSocket, code: Int, reason: String) { webSocket.close(1000, null); closed.complete(Unit) }
                    override fun onClosed(webSocket: WebSocket, code: Int, reason: String) { closed.complete(Unit) }
                    override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) { closed.complete(Unit) }
                })
            try {
                // A ping every 30 seconds keeps the route open; the service answers without waking up.
                while (!closed.isCompleted) {
                    if (withTimeoutOrNull(30_000) { closed.await() } == null && !socket.send("ping")) break
                }
            } finally { socket.cancel() }
            if (System.currentTimeMillis() - opened > 60_000) wait = 1_000L
            delay(wait)
            wait = (wait * 2).coerceAtMost(30_000L)
        }
    }
}
