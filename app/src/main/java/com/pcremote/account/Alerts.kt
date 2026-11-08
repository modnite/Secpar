package com.pcremote.account

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.content.ContextCompat
import com.pcremote.MainActivity
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** One notification with two answer buttons. [id] is the friend's account id or the request id. */
data class Alert(val id: String, val title: String, val text: String, val yes: Answer, val no: Answer)

/**
 * Decides which friend requests and connection requests are new. Friend requests that were already
 * waiting when the first poll ran are not announced, because they carry no time. Connection requests
 * expire after five minutes, so those are always announced.
 */
class AlertTracker {
    private val friends = mutableSetOf<String>()
    private val requests = mutableSetOf<String>()
    private var primed = false

    fun next(ownId: String, friendList: List<Friend>, requestList: List<ConnectionRequest>): List<Alert> {
        val out = mutableListOf<Alert>()
        for (f in friendList.filter { it.pending && it.requester != ownId }) {
            if (friends.add(f.id) && primed) out += Alert(f.id, "Friend request", "@${f.handle} wants to be your friend", Answer.ACCEPT_FRIEND, Answer.DECLINE_FRIEND)
        }
        primed = true
        for (r in FriendsModel.waiting(requestList, ownId)) {
            if (requests.add(r.id)) out += Alert(r.id, "Connection request", FriendsModel.requestText(r), Answer.ALLOW_CONNECTION, Answer.DENY_CONNECTION)
        }
        return out
    }
}

private const val CHANNEL_WATCH = "secpar_watch"
private const val CHANNEL_REQUESTS = "secpar_requests"
private const val WATCH_ID = 7301
private const val POLL_MS = 15_000L

/**
 * Watches for new requests while the user has turned notifications on. There is no push service, so
 * it asks the account service every 15 seconds and shows a persistent low priority notice, which
 * Android requires for a service that keeps running.
 */
class AlertService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var started = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val nm = getSystemService(NotificationManager::class.java)
        channels(nm)
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        val watching = Notification.Builder(this, CHANNEL_WATCH).setSmallIcon(android.R.drawable.stat_notify_sync_noanim)
            .setContentTitle("Secpar").setContentText("Watching for requests").setContentIntent(open).setOngoing(true).build()
        if (Build.VERSION.SDK_INT >= 34) startForeground(WATCH_ID, watching, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        else startForeground(WATCH_ID, watching)
        if (!started) {
            started = true
            scope.launch { loop(nm) }
        }
        return START_NOT_STICKY
    }

    private suspend fun loop(nm: NotificationManager) {
        val controller = AccountRuntime.get(this).controller
        val tracker = AlertTracker()
        while (true) {
            val own = controller.state.value.user?.id
            if (own != null) controller.poll()?.let { (friends, requests) ->
                tracker.next(own, friends, requests).forEach { post(nm, it) }
                // A request that someone answered elsewhere stops asking.
                val open = requests.filter { it.state == "pending" }.map { it.id } + friends.filter { it.pending }.map { it.id }
                nm.activeNotifications.filter { it.id != WATCH_ID && it.tag != null && it.tag !in open }.forEach { nm.cancel(it.tag, it.id) }
            }
            delay(POLL_MS)
        }
    }

    private fun post(nm: NotificationManager, alert: Alert) {
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        fun action(answer: Answer, label: String): Notification.Action {
            val intent = Intent(this, AlertReceiver::class.java).setAction("com.pcremote.ANSWER")
                .putExtra("answer", answer.name).putExtra("id", alert.id)
            val pending = PendingIntent.getBroadcast(this, (alert.id + answer.name).hashCode(), intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            return Notification.Action.Builder(null, label, pending).build()
        }
        val n = Notification.Builder(this, CHANNEL_REQUESTS).setSmallIcon(android.R.drawable.stat_notify_chat)
            .setContentTitle(alert.title).setContentText(alert.text).setContentIntent(open).setAutoCancel(true)
            .setTimeoutAfter(5 * 60_000L)
            .addAction(action(alert.yes, label(alert.yes))).addAction(action(alert.no, label(alert.no))).build()
        nm.notify(alert.id, 1, n)
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    companion object {
        fun label(answer: Answer) = when (answer) {
            Answer.ALLOW_CONNECTION -> FriendsModel.ALLOW
            Answer.DENY_CONNECTION -> FriendsModel.DENY
            Answer.ACCEPT_FRIEND -> FriendsModel.ACCEPT
            Answer.DECLINE_FRIEND -> "Decline"
        }

        private fun channels(nm: NotificationManager) {
            nm.createNotificationChannel(NotificationChannel(CHANNEL_WATCH, "Watching", NotificationManager.IMPORTANCE_MIN))
            nm.createNotificationChannel(NotificationChannel(CHANNEL_REQUESTS, "Requests", NotificationManager.IMPORTANCE_HIGH))
        }

        /** Starts or stops the watcher. Start it from a visible screen, because Android blocks it from the background. */
        fun sync(ctx: Context, on: Boolean) {
            val intent = Intent(ctx, AlertService::class.java)
            if (on) runCatching { ContextCompat.startForegroundService(ctx, intent) } else ctx.stopService(intent)
        }
    }
}

/** The Allow and Deny buttons on a notification. */
class AlertReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val answer = runCatching { Answer.valueOf(intent.getStringExtra("answer") ?: "") }.getOrNull() ?: return
        val id = intent.getStringExtra("id")?.takeIf { AccountText.UUID_ID.matches(it) } ?: return
        val pending = goAsync()
        CoroutineScope(SupervisorJob() + Dispatchers.Main).launch {
            try {
                AccountRuntime.get(context).controller.answerNow(answer, id)
                context.getSystemService(NotificationManager::class.java).cancel(id, 1)
            } finally { pending.finish() }
        }
    }
}
