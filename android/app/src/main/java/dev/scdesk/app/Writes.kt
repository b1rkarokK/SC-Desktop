package dev.scdesk.app

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject

/**
 * Likes, follows and playlists written from the phone. SoundCloud's anti-bot check may refuse the phone: then the
 * computer, if it is connected, does the same through its own browser session. The call has the shape of `Link.rpc`.
 */
object Writes {
    /** «Лайкнуть всё»: from the last song to the first (the likes list shows the newest first). The phone itself; the computer for the rest when SoundCloud refuses. */
    suspend fun likeMany(app: App, ids: List<Long>) {
        val order = ids.reversed()
        var done = 0
        for (id in order) {
            val ok = runCatching { withContext(Dispatchers.IO) { app.api.likeTrack(id, true) } }.isSuccess
            if (!ok) break
            done++
            kotlinx.coroutines.delay(120)
        }
        if (done == order.size) return
        if (!app.store.linked) throw java.io.IOException("SoundCloud отказал, нужен подключённый компьютер")
        // the rest, in the same order, by the computer (it reverses a list itself: give it the first ones)
        app.link.rpc("likes_many", JSONObject().put("tracks", org.json.JSONArray(order.drop(done).reversed())), waitMs = 300_000)
    }

    suspend fun call(app: App, op: String, args: JSONObject): JSONObject {
        val direct = runCatching {
            withContext(Dispatchers.IO) {
                when (op) {
                    "like" -> app.api.likeTrack(args.getLong("track"), args.optBoolean("on", true))
                    "follow" -> app.api.followUser(args.getLong("id"), args.optBoolean("on", true))
                    "pl_add" -> app.api.addToPlaylist(args.getLong("playlist"), args.getLong("track"))
                    "pl_remove" -> app.api.removeFromPlaylist(args.getLong("playlist"), args.getLong("track"))
                    "pl_create" -> app.api.createPlaylist(args.getString("title"), if (args.has("track")) args.getLong("track") else null)
                    "pl_delete" -> app.api.deletePlaylist(args.getLong("playlist"))
                    else -> throw IllegalArgumentException(op)
                }
            }
        }
        if (direct.isSuccess) return JSONObject().put("ok", true)
        // a clear refusal of the thing itself (already in the playlist) is told as it is; the rest goes to the computer
        val e = direct.exceptionOrNull()
        if (e?.message?.contains("уже есть") == true) throw e
        if (app.store.linked) return app.link.rpc(op, args)
        throw java.io.IOException(L.t("SoundCloud не принял запрос с телефона. Подключите компьютер: он отправит его от своего имени."))
    }
}
