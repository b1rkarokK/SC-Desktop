package dev.scdesk.app

import okhttp3.MediaType.Companion.toMediaType
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject
import java.time.LocalDate
import java.time.ZoneOffset
import java.util.UUID

/**
 * How many people use the app: an anonymous count, nothing more (the same as the computer's src-tauri/src/stats.rs).
 * Once a day it tells the sync server a random number of this installation (made here), the version and the system.
 * Not sent: a name, the account, an address, what is played. It can be switched off in Settings.
 */
object Stats {
    /** Sends the number if it is on and has not been sent today (blocking: the network). */
    fun maybeSend(store: Store) {
        if (!store.statsAnon) return
        val today = LocalDate.now(ZoneOffset.UTC).toString()
        if (store.statsDay == today) return
        // a debug build is the developer's own: not a user
        if (BuildConfig.DEBUG) return
        val id = store.installId.takeIf { it.length == 32 } ?: UUID.randomUUID().toString().replace("-", "")
        val body = JSONObject().put("id", id).put("v", BuildConfig.VERSION_NAME).put("os", "android").toString()
        val ok = runCatching {
            http.newCall(Request.Builder().url("$RELAY/stat").header("User-Agent", USER_AGENT).post(body.toRequestBody("application/json".toMediaType())).build()).execute().use { it.isSuccessful }
        }.getOrDefault(false)
        if (ok) {
            store.installId = id
            store.statsDay = today
        }
    }
}
