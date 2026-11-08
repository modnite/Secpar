package com.pcremote.account

import android.content.Context
import android.os.Build
import com.pcremote.agent.KeyValue
import com.pcremote.agent.KeystoreBox
import com.pcremote.agent.PairedPc
import com.pcremote.agent.agentPair
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import java.security.SecureRandom

/** One process-wide account context. Preferences are excluded from backup/device transfer. */
class AccountRuntime private constructor(ctx: Context) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val prefs = ctx.applicationContext.getSharedPreferences("secpar_account", Context.MODE_PRIVATE)
    val store = AccountStore(object : KeyValue {
        override fun get(key: String) = prefs.getString(key, null)
        override fun put(key: String, value: String?) {
            check(prefs.edit().putString(key, value).commit()) { "Could not save account access" }
        }
    }, KeystoreBox)
    val controller = AccountController(store, scope, "Secpar Android (${Build.MODEL})", pair = { uri ->
        val pin = "%04d".format(java.util.Locale.ROOT, SecureRandom().nextInt(10_000))
        val paired = agentPair(scope, uri, Build.MODEL, pin, preferLan = true).getOrThrow()
        PairedPc(paired.pc.id, paired.pc.name, uri.hosts, uri.port, uri.fingerprint, paired.token, paired.pc.macs, relay = paired.relay)
    })
    /** Request notifications are off until the user turns them on. */
    val alerts = kotlinx.coroutines.flow.MutableStateFlow(prefs.getBoolean("alerts", false))
    fun setAlerts(on: Boolean) {
        prefs.edit().putBoolean("alerts", on).apply()
        alerts.value = on
    }
    val access = OwnedPcAccess(store) { controller.state.value.user?.id }

    companion object {
        @Volatile private var instance: AccountRuntime? = null
        fun get(ctx: Context): AccountRuntime = instance ?: synchronized(this) {
            instance ?: AccountRuntime(ctx.applicationContext).also { instance = it }
        }
    }
}
