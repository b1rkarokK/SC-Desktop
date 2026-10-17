package dev.scdesk.app

import android.net.Uri
import org.json.JSONObject
import java.util.Base64

/**
 * The code the computer shows: scdesk://pair?c=<channel>&k=<key, base64url>[&m=login].
 * With `m=login` the computer has no account and asks this phone for its own ([take]).
 */
class PairCode(val chan: String, val key: ByteArray, val take: Boolean = false) {
    companion object {
        fun parse(text: String): PairCode? = try {
            val u = Uri.parse(text.trim())
            val c = u.getQueryParameter("c")
            val k = u.getQueryParameter("k")
            if (u.scheme != "scdesk" || u.host != "pair" || c.isNullOrEmpty() || k.isNullOrEmpty()) null
            else PairCode(c, Base64.getUrlDecoder().decode(k), u.getQueryParameter("m") == "login").takeIf { it.key.size == 32 }
        } catch (e: Exception) {
            null
        }
    }
}

/**
 * Joins the pairing channel, says hello and waits for the computer's answer (after its owner allows it): the login
 * of the account and the key of the permanent link. Saved into [store].
 */
class Pairing(private val store: Store, private val code: PairCode, private val done: (Result<Unit>) -> Unit) {
    private var channel: Channel? = null
    @Volatile private var finished = false

    fun start() {
        // the computer asks for an account: this phone has to have one to give
        if (code.take && !store.signedIn) {
            finish(Result.failure(IllegalStateException("На телефоне ещё нет входа. Сначала войдите в SoundCloud, потом отсканируйте код.")))
            return
        }
        android.util.Log.i("SCDesk", "pairing: joining ${code.chan}")
        val ch = Channel(code.chan, code.key, { m -> handle(m) }, onUp = { hello() })
        channel = ch
        ch.start()
        // the computer waits five minutes; so do we, a little less
        Thread {
            Thread.sleep(240_000)
            if (!finished) finish(Result.failure(IllegalStateException("Компьютер не ответил. Нажмите «Подключить телефон» на компьютере ещё раз.")))
        }.apply { isDaemon = true }.start()
    }

    private fun hello() {
        android.util.Log.i("SCDesk", "pairing: hello sent")
        if (finished) return
        if (code.take) {
            // the computer waits for the account: it asks its owner before it takes it
            channel?.send(
                JSONObject().put("t", "account").put("dev", store.deviceId).put("name", store.deviceName)
                    .put("creds", JSONObject().put("client_id", store.clientId).put("oauth_token", store.oauthToken).put("stream_token", store.streamToken.ifEmpty { null }))
                    .put("me", JSONObject().put("id", store.userId).put("username", store.userName)),
            )
        } else {
            channel?.send(JSONObject().put("t", "hello").put("dev", store.deviceId).put("name", store.deviceName))
        }
    }

    private fun handle(m: JSONObject) {
        if (m.optString("t") != "link" || m.optString("phone") != store.deviceId || finished) return
        try {
            val link = m.getJSONObject("link")
            val pc = m.getJSONObject("pc")
            // the computer gave the account (a phone without one); a phone that gave its own gets only the link back
            if (m.has("creds")) {
                val creds = m.getJSONObject("creds")
                val me = m.getJSONObject("me")
                store.clientId = creds.getString("client_id")
                store.oauthToken = creds.getString("oauth_token")
                store.streamToken = if (creds.isNull("stream_token")) "" else creds.optString("stream_token", "")
                store.userId = me.getLong("id")
                store.userName = me.optString("username")
            }
            store.linkKey = link.getString("key")
            store.pcDev = pc.getString("dev")
            store.pcName = pc.optString("name")
            store.linkChan = link.getString("chan")
            finish(Result.success(Unit))
        } catch (e: Exception) {
            finish(Result.failure(e))
        }
    }

    private fun finish(r: Result<Unit>) {
        if (finished) return
        finished = true
        channel?.close()
        done(r)
    }

    fun cancel() {
        finished = true
        channel?.close()
    }
}
