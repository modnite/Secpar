package com.pcremote.home

import android.content.Context
import com.pcremote.agent.KeyValue
import com.pcremote.agent.KeystoreBox
import com.pcremote.agent.SecretBox
import com.pcremote.settings.prefsKv
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.add
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import java.util.concurrent.TimeUnit

enum class WakeMethod { LOCAL, WEBHOOK, HOME_ASSISTANT }

/** Optional remote helper. Local magic packets always work independently of this setting. */
class RemoteWake(private val kv: KeyValue, private val box: SecretBox = KeystoreBox) {
    private val ha get() = HomeAssistant(kv, box)
    var method: WakeMethod
        get() = kv.get("secpar_wake_method")?.let { runCatching { WakeMethod.valueOf(it) }.getOrNull() }
            ?: if (ha.configured) WakeMethod.HOME_ASSISTANT else WakeMethod.LOCAL
        set(value) = kv.put("secpar_wake_method", value.name)
    var url: String
        get() = secret("secpar_wake_url")
        set(value) = saveSecret("secpar_wake_url", value.trim())
    var token: String
        get() = secret("secpar_wake_token")
        set(value) = saveSecret("secpar_wake_token", value.trim())
    private fun secret(key: String) = kv.get(key)?.let { runCatching { box.open(it) }.getOrNull() }.orEmpty()
    private fun saveSecret(key: String, value: String) = kv.put(key, value.takeIf { it.isNotEmpty() }?.let(box::seal))

    fun wake(macs: List<String>): Result<Unit> = when (method) {
        WakeMethod.LOCAL -> Result.failure(IllegalStateException("Local wake uses magic packets"))
        WakeMethod.HOME_ASSISTANT -> ha.wake(macs)
        WakeMethod.WEBHOOK -> runCatching {
            client.newCall(request(url, token, macs)).execute().use {
                check(it.isSuccessful) { "Wake helper did not accept the request" }
            }
        }
    }
    companion object {
        private val client by lazy { OkHttpClient.Builder().followRedirects(false).followSslRedirects(false)
            .connectTimeout(8, TimeUnit.SECONDS).readTimeout(10, TimeUnit.SECONDS).build() }
        fun get(ctx: Context) = RemoteWake(prefsKv(ctx))
        fun validUrl(value: String): Boolean = runCatching {
            val u = value.trim().toHttpUrl()
            u.isHttps && u.username.isEmpty() && u.password.isEmpty() && u.fragment == null
        }.getOrDefault(false)
        fun request(url: String, token: String, macs: List<String>): Request {
            require(validUrl(url)) { "Use an HTTPS wake link" }
            val body = buildJsonObject { put("macs", buildJsonArray { macs.forEach { add(it) } }) }
            return Request.Builder().url(url.trim())
                .apply { if (token.isNotBlank()) header("Authorization", "Bearer ${token.trim()}") }
                .post(body.toString().toRequestBody("application/json".toMediaType())).build()
        }
    }
}
