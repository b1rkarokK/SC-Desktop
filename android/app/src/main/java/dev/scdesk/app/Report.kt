package dev.scdesk.app

import android.content.Context
import android.content.Intent
import android.os.Build

/**
 * «Отчёт об ошибке»: the version, the phone and the last lines of the app's own log, put into a text that is sent by
 * what the user picks (a message, mail, a note). Nothing leaves by itself.
 */
object Report {
    fun build(): String {
        val log = runCatching {
            // an app may read only its own log lines
            Runtime.getRuntime().exec(arrayOf("logcat", "-d", "-t", "300", "-s", "SCDesk:*", "AndroidRuntime:E")).inputStream.bufferedReader().readText().takeLast(12_000)
        }.getOrDefault("")
        return buildString {
            appendLine("SC Desk ${BuildConfig.VERSION_NAME} (${BuildConfig.VERSION_CODE})")
            appendLine("Android ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT}), ${Build.MANUFACTURER} ${Build.MODEL}, ${Build.SUPPORTED_ABIS.firstOrNull()}")
            appendLine("Язык: ${L.lang}, сеть: ${if (Tunnel.port != 0) "через сервер" else "обычная"}")
            appendLine()
            appendLine(log.ifBlank { "(журнал пуст)" })
        }
    }

    fun send(ctx: Context) {
        val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_SUBJECT, "SC Desk: отчёт об ошибке").putExtra(Intent.EXTRA_TEXT, build())
        ctx.startActivity(Intent.createChooser(send, null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }
}
