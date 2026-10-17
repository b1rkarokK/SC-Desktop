package dev.scdesk.app

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.net.Uri
import android.os.Build
import android.provider.Settings
import okhttp3.Request
import org.json.JSONObject
import java.io.File
import java.io.IOException
import java.security.MessageDigest

/** A newer build: what the feed (latest-android.json of the GitHub release) says about it. */
data class UpdateInfo(val versionName: String, val versionCode: Int, val notes: String, val url: String, val sha256: String, val size: Long)

/**
 * Updating from inside the app, the way the computer's SC Desk does: it sees a new release, shows what changed,
 * the user presses «Обновить», the package is downloaded (checked by its SHA-256), installed by the system's installer
 * and the app says it is updated. What was downloaded does not stay: it is deleted after the install and at every start.
 */
object Updater {
    const val FEED = "https://github.com/b1rkarokK/SC-Desktop/releases/latest/download/latest-android.json"

    fun parse(text: String): UpdateInfo? = try {
        val j = JSONObject(text)
        val apk = j.getJSONObject("apk")
        UpdateInfo(j.getString("version"), j.getInt("versionCode"), j.optString("notes"), apk.getString("url"), apk.getString("sha256").lowercase(), apk.optLong("size"))
    } catch (e: Exception) {
        null
    }

    /** A newer build than [currentCode], or null (blocking: the network). */
    fun check(currentCode: Int, feed: String = FEED): UpdateInfo? {
        val text = http.newCall(Request.Builder().url(feed).header("User-Agent", USER_AGENT).build()).execute().use { r ->
            if (!r.isSuccessful) throw IOException("update feed ${r.code}")
            r.body?.string() ?: return null
        }
        return parse(text)?.takeIf { it.versionCode > currentCode && it.url.startsWith("https://") }
    }

    private fun dir(ctx: Context) = File(ctx.cacheDir, "update")

    /** Downloads the package (blocking), checks its SHA-256, and gives the file. */
    fun download(ctx: Context, info: UpdateInfo, progress: (Float) -> Unit): File {
        cleanup(ctx)
        val out = File(dir(ctx).also { it.mkdirs() }, "sc-desk.apk")
        val md = MessageDigest.getInstance("SHA-256")
        http.newBuilder().readTimeout(60, java.util.concurrent.TimeUnit.SECONDS).build()
            .newCall(Request.Builder().url(info.url).header("User-Agent", USER_AGENT).build()).execute().use { r ->
                if (!r.isSuccessful) throw IOException("download ${r.code}")
                val body = r.body ?: throw IOException("empty answer")
                val total = body.contentLength().takeIf { it > 0 } ?: info.size
                var got = 0L
                body.byteStream().use { input ->
                    out.outputStream().use { o ->
                        val buf = ByteArray(64 * 1024)
                        while (true) {
                            val n = input.read(buf)
                            if (n < 0) break
                            o.write(buf, 0, n)
                            md.update(buf, 0, n)
                            got += n
                            if (total > 0) progress((got.toFloat() / total).coerceIn(0f, 1f))
                        }
                    }
                }
            }
        val sum = md.digest().joinToString("") { "%02x".format(it) }
        if (sum != info.sha256) {
            out.delete()
            throw IOException("the package does not match its checksum")
        }
        return out
    }

    /** The system may need the user's say once to let this app install packages: the settings page for it. */
    fun mayInstall(ctx: Context): Boolean = ctx.packageManager.canRequestPackageInstalls()

    fun askToAllow(ctx: Context) {
        ctx.startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${ctx.packageName}")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }

    /** Hands the package to the system installer; the answer comes to [InstallReceiver]. */
    fun install(ctx: Context, file: File) {
        val installer = ctx.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL)
        // on Android 12 and later an update of an app that installed itself goes without a question
        if (Build.VERSION.SDK_INT >= 31) params.setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED)
        val id = installer.createSession(params)
        installer.openSession(id).use { s ->
            file.inputStream().use { i -> s.openWrite("sc-desk", 0, file.length()).use { o -> i.copyTo(o); s.fsync(o) } }
            val pending = PendingIntent.getBroadcast(ctx, id, Intent(ctx, InstallReceiver::class.java).setPackage(ctx.packageName), PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            s.commit(pending.intentSender)
        }
    }

    /** What was downloaded is not kept. */
    fun cleanup(ctx: Context) {
        dir(ctx).deleteRecursively()
    }

    fun channel(ctx: Context): String {
        val id = "updates"
        val nm = ctx.getSystemService(NotificationManager::class.java)
        if (nm.getNotificationChannel(id) == null) nm.createNotificationChannel(NotificationChannel(id, "Обновления", NotificationManager.IMPORTANCE_DEFAULT))
        return id
    }
}

/** The answer of the installer: if it wants the user's tap, the window is opened; when it is done, the package is deleted. */
class InstallReceiver : BroadcastReceiver() {
    override fun onReceive(ctx: Context, intent: Intent) {
        when (intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                @Suppress("DEPRECATION")
                val ask = intent.getParcelableExtra<Intent>(Intent.EXTRA_INTENT)
                ask?.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)?.let { ctx.startActivity(it) }
            }
            PackageInstaller.STATUS_SUCCESS -> Updater.cleanup(ctx)
            else -> {
                Updater.cleanup(ctx)
                android.util.Log.w("SCDesk", "update: ${intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE)}")
            }
        }
    }
}

/** The new version has just replaced the old one: it says so, and one tap opens it (the system closes the app on an update). */
class UpdatedReceiver : BroadcastReceiver() {
    override fun onReceive(ctx: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_MY_PACKAGE_REPLACED) return
        Updater.cleanup(ctx)
        val open = PendingIntent.getActivity(ctx, 0, Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK), PendingIntent.FLAG_IMMUTABLE)
        val n = androidx.core.app.NotificationCompat.Builder(ctx, Updater.channel(ctx))
            .setSmallIcon(R.drawable.ic_stat_sc)
            .setContentTitle("SC Desk обновлён")
            .setContentText("Версия ${BuildConfig.VERSION_NAME}. Нажмите, чтобы открыть.")
            .setContentIntent(open).setAutoCancel(true).build()
        try {
            ctx.getSystemService(NotificationManager::class.java).notify(2001, n)
        } catch (e: SecurityException) {
            // no permission to show it: the app opens by hand
        }
    }
}
