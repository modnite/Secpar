package com.pcremote.update

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.content.BroadcastReceiver
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.net.Uri
import android.os.Build
import android.provider.Settings
import com.pcremote.BuildConfig
import com.pcremote.R
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import okhttp3.OkHttpClient
import okhttp3.Request
import java.io.File

data class Release(val version: String, val apkUrl: String, val sha256: String = "", val size: Long = 0)
const val CHECKING = "Checking for updates…"
const val UP_TO_DATE = "You’re up to date."
const val CHECK_FAILED = "Could not check for updates. Try again."

data class UpdateState(val release: Release? = null, val message: String = CHECKING, val busy: Boolean = false)

/** Automatic discovery; installation always requires an explicit user action. */
object Updater {
    private const val LATEST = "https://api.github.com/repos/modnite/Secpar/releases?per_page=100"
    private const val JOB = 47_01
    private const val CHANNEL = "updates"
    private val http by lazy { OkHttpClient.Builder().callTimeout(3, java.util.concurrent.TimeUnit.MINUTES).build() }
    private val mutableState = kotlinx.coroutines.flow.MutableStateFlow(UpdateState())
    val state: kotlinx.coroutines.flow.StateFlow<UpdateState> = mutableState
    private val pendingConfirmation = kotlinx.coroutines.flow.MutableStateFlow<Intent?>(null)
    val confirmation: kotlinx.coroutines.flow.StateFlow<Intent?> = pendingConfirmation
    fun clearConfirmation() { pendingConfirmation.value = null }
    internal fun confirm(intent: Intent) { pendingConfirmation.value = intent }
    private val operation = java.util.concurrent.atomic.AtomicBoolean(false)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    /** Select only this product's channel, never desktop/host releases. */
    fun parse(json: String): Release? = runCatching {
        val o = Json.parseToJsonElement(json).jsonObject
        if (o["draft"]?.jsonPrimitive?.content == "true") return null
        val tag = o["tag_name"]!!.jsonPrimitive.content
        if (!Regex("v\\d+\\.\\d+\\.\\d+(?:-secpar-preview\\.\\d+)?").matches(tag)) return null
        val apk = o["assets"]!!.jsonArray.map { it.jsonObject }
            .first { Regex("Secpar(?:-preview[0-9]+)?\\.apk").matches(it["name"]!!.jsonPrimitive.content) }
        val url = apk["browser_download_url"]!!.jsonPrimitive.content
        val prefix = "https://github.com/modnite/Secpar/releases/download/$tag/"
        if (!url.startsWith(prefix) || url.removePrefix(prefix).contains('/')) return null
        val hash = apk["digest"]!!.jsonPrimitive.content.removePrefix("sha256:")
        if (!Regex("[a-f0-9]{64}").matches(hash)) return null
        val size = apk["size"]!!.jsonPrimitive.content.toLong()
        if (size !in 1..200_000_000) return null
        Release(tag.trimStart('v'), url, hash, size)
    }.getOrNull()

    fun select(json: String, previews: Boolean = true): Release? = runCatching {
        Json.parseToJsonElement(json).jsonArray.mapNotNull { parse(it.toString()) }
            .filter { previews || !it.version.contains("-secpar-preview.") }
            .reduceOrNull { a, b -> if (isNewer(b.version, a.version)) b else a }
    }.getOrNull()

    fun isNewer(latest: String, current: String): Boolean {
        fun v(s: String): List<Long> {
            val clean = s.trimStart('v')
            val base = clean.substringBefore('-').split('.').map { it.toLongOrNull() ?: 0 }
            val preview = clean.substringAfter("-secpar", "").trimStart('-', '.').removePrefix("preview.").toLongOrNull()
            return (base + listOf(0L, 0L, 0L)).take(3) + listOf(if (preview == null) 1L else 0L, preview ?: 0)
        }
        val a=v(latest); val b=v(current)
        for (i in a.indices) if (a[i] != b[i]) return a[i] > b[i]
        return false
    }

    /** Every 6 h while the phone has a network, plus once now. Debug builds never self-update. */
    fun schedule(ctx: Context) {
        if (BuildConfig.DEBUG) return
        val js = ctx.getSystemService(JobScheduler::class.java)
        if (js.getPendingJob(JOB) == null) {
            js.schedule(JobInfo.Builder(JOB, ComponentName(ctx, UpdateJob::class.java))
                .setRequiredNetworkType(JobInfo.NETWORK_TYPE_ANY)
                .setPeriodic(6 * 3600 * 1000L)
                .setPersisted(true)
                .build())
        }
        checkNow(ctx)
    }

    fun checkNow(ctx: Context, done: () -> Unit = {}) {
        if (!operation.compareAndSet(false, true)) return done()
        mutableState.value = mutableState.value.copy(message = CHECKING, busy = true)
        scope.launch {
            try {
                val body = http.newCall(Request.Builder().url(LATEST).header("Accept", "application/vnd.github+json").build()).execute()
                    .use { check(it.isSuccessful) { "Update service unavailable. Try again later." }; it.body.string() }
                val r = select(body, BuildConfig.VERSION_NAME.contains("secpar"))
                val newer = r?.takeIf { isNewer(it.version, BuildConfig.VERSION_NAME) }
                mutableState.value = UpdateState(newer, if (newer == null) UP_TO_DATE else "Secpar ${newer.version} is available.")
                if (newer != null) notify(ctx, "Secpar ${newer.version} is available. Tap to review.", Intent(ctx, UpdatesActivity::class.java))
            } catch (_: Exception) {
                mutableState.value = mutableState.value.copy(message = CHECK_FAILED, busy = false)
            } finally {
                operation.set(false)
                android.os.Handler(android.os.Looper.getMainLooper()).post { done() }
            }
        }
    }

    fun downloadAndInstall(ctx: Context) {
        val r = state.value.release ?: return
        if (Build.VERSION.SDK_INT >= 26 && !ctx.packageManager.canRequestPackageInstalls()) {
            ctx.startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:" + ctx.packageName)))
            mutableState.value = state.value.copy(message = "Allow updates from Secpar, then tap Download update again.")
            return
        }
        if (!operation.compareAndSet(false, true)) return
        mutableState.value = state.value.copy(busy = true, message = "Downloading update…")
        scope.launch {
            val apk = File(ctx.cacheDir, "secpar-update.apk")
            try {
                val digest = java.security.MessageDigest.getInstance("SHA-256")
                var total = 0L
                http.newCall(Request.Builder().url(r.apkUrl).build()).execute().use { resp ->
                    check(resp.isSuccessful) { "Download failed" }
                    apk.outputStream().use { out -> resp.body.byteStream().use { input ->
                        val buffer = ByteArray(65536)
                        while (true) {
                            val n = input.read(buffer); if (n < 0) break
                            total += n; check(total <= r.size) { "Unexpected download size" }
                            digest.update(buffer, 0, n); out.write(buffer, 0, n)
                        }
                    } }
                }
                check(total == r.size && digest.digest().joinToString("") { "%02x".format(it) } == r.sha256) { "Download verification failed" }
                val flags = if (Build.VERSION.SDK_INT >= 28) android.content.pm.PackageManager.GET_SIGNING_CERTIFICATES else @Suppress("DEPRECATION") android.content.pm.PackageManager.GET_SIGNATURES
                val downloaded = ctx.packageManager.getPackageArchiveInfo(apk.path, flags) ?: error("Invalid APK")
                val installed = ctx.packageManager.getPackageInfo(ctx.packageName, flags)
                fun version(p: android.content.pm.PackageInfo) = if (Build.VERSION.SDK_INT >= 28) p.longVersionCode else @Suppress("DEPRECATION") p.versionCode.toLong()
                fun signers(p: android.content.pm.PackageInfo) = (if (Build.VERSION.SDK_INT >= 28) p.signingInfo!!.apkContentsSigners else @Suppress("DEPRECATION") p.signatures!!).map { it.toCharsString() }.toSet()
                check(downloaded.packageName == ctx.packageName && version(downloaded) > version(installed)) { "Wrong app or older update" }
                check(signers(installed).isNotEmpty() && signers(installed) == signers(downloaded)) { "Update signing key does not match" }
                install(ctx, apk)
                mutableState.value = state.value.copy(busy = false, message = "Download verified. Confirm installation when Android asks.")
            } catch (_: Exception) {
                mutableState.value = state.value.copy(busy = false, message = "Update could not be verified or installed. Try again.")
            } finally { apk.delete(); operation.set(false) }
        }
    }

    private fun install(ctx: Context, apk: File) {
        val pi = ctx.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
            setAppPackageName(ctx.packageName)
            if (Build.VERSION.SDK_INT >= 31) setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_REQUIRED)
        }
        val id = pi.createSession(params)
        pi.openSession(id).use { s ->
            s.openWrite("base.apk", 0, apk.length()).use { out -> apk.inputStream().use { it.copyTo(out) }; s.fsync(out) }
            val status = PendingIntent.getBroadcast(ctx, id, Intent(ctx, UpdateReceiver::class.java),
                PendingIntent.FLAG_UPDATE_CURRENT or (if (Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else 0))
            s.commit(status.intentSender)
        }
    }

    internal fun notify(ctx: Context, text: String, action: Intent) {
        val nm = ctx.getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= 26) nm.createNotificationChannel(NotificationChannel(CHANNEL, ctx.getString(R.string.pcr_update_channel), NotificationManager.IMPORTANCE_DEFAULT))
        val tap = PendingIntent.getActivity(ctx, 1, action.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val b = if (Build.VERSION.SDK_INT >= 26) Notification.Builder(ctx, CHANNEL) else @Suppress("DEPRECATION") Notification.Builder(ctx)
        nm.notify(4702, b.setSmallIcon(android.R.drawable.stat_sys_download_done).setContentTitle(ctx.getString(R.string.pcr_update_title))
            .setContentText(text).setContentIntent(tap).setAutoCancel(true).build())
    }
}

/** Installer results: asks for the one confirmation Android may still want. */
class UpdateReceiver : BroadcastReceiver() {
    override fun onReceive(ctx: Context, intent: Intent) {
        when (intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                @Suppress("DEPRECATION") val confirm = intent.getParcelableExtra<Intent>(Intent.EXTRA_INTENT) ?: return
                Updater.confirm(confirm)
                Updater.notify(ctx, ctx.getString(R.string.pcr_update_tap), confirm)
            }
            PackageInstaller.STATUS_SUCCESS -> {}
            else -> Updater.notify(ctx, "Update was not installed. Tap to retry.", Intent(ctx, UpdatesActivity::class.java))
        }
    }
}

class UpdateJob : JobService() {
    override fun onStartJob(params: JobParameters): Boolean {
        Updater.checkNow(this) { jobFinished(params, false) }
        return true
    }

    override fun onStopJob(params: JobParameters) = true
}
