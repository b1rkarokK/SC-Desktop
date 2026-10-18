package dev.scdesk.app

import android.os.SystemClock
import kotlinx.coroutines.flow.MutableStateFlow
import org.json.JSONArray
import org.json.JSONObject
import java.util.Base64
import kotlin.concurrent.thread

data class Peer(
    val dev: String,
    val name: String,
    val kind: String,
    /** when we heard it last (SystemClock) */
    val seenAt: Long,
    val track: Track?,
    val playing: Boolean,
    val posMs: Long,
    /** when [posMs] was told (SystemClock): the position counts on from there */
    val posAt: Long,
    val volume: Float,
)

/**
 * The link of this phone with the computer (and any other device of the account), over the relay: the same
 * messages as src-tauri/src/devices.rs. The connection is kept only while the app is on the screen or the music
 * plays, so that it costs the battery nothing otherwise.
 */
class Link(private val app: App) {
    val peers = MutableStateFlow<Map<String, Peer>>(emptyMap())

    private var channel: Channel? = null
    private var foreground = false
    private var told = Triple<Long?, Boolean, Int>(null, false, 100)
    private var toldAt = 0L

    private val store get() = app.store

    /** the app is on the screen / left the screen */
    fun setForeground(on: Boolean) {
        foreground = on
        refreshConnection()
    }

    /** Connected while it is needed: the screen is on or the music plays. */
    @Synchronized
    fun refreshConnection() {
        val need = store.linked && (foreground || app.engine.state.value.playing)
        if (need && channel == null) {
            val key = Base64.getDecoder().decode(store.linkKey)
            channel = Channel(store.linkChan, key, ::onMessage, onUp = { present(); tellState(true) }).also { it.start() }
        } else if (!need && channel != null) {
            channel?.close()
            channel = null
        }
    }

    @Synchronized
    fun stop() {
        channel?.close()
        channel = null
        peers.value = emptyMap()
    }

    private fun send(msg: JSONObject) {
        synchronized(this) { channel }?.send(msg)
    }

    fun present() {
        send(JSONObject().put("t", "pres").put("dev", store.deviceId).put("name", store.deviceName).put("kind", "phone").put("ts", System.currentTimeMillis()))
    }

    /** The player changed (told by the engine on every event): the others hear of a new track, play / pause, volume. */
    fun onPlayerChanged() {
        refreshConnection()
        val s = app.engine.state.value
        val key = Triple(s.track?.id, s.playing, (s.volume * 100).toInt())
        val now = SystemClock.elapsedRealtime()
        if (key != told || (s.playing && now - toldAt >= 5_000)) {
            told = key
            toldAt = now
            tellState(false)
        }
    }

    fun tellState(force: Boolean) {
        app.engine.run { c ->
            val s = app.engine.state.value
            if (!force && s.track == null) return@run
            send(
                JSONObject().put("t", "st").put("dev", store.deviceId).put("ts", System.currentTimeMillis())
                    .put("track", s.track?.let { JSONObject().put("id", it.id).put("title", it.title).put("artist", it.artist).put("dur", it.durationMs).put("art", it.artwork) })
                    .put("pos", c.currentPosition).put("playing", s.playing).put("vol", s.volume.toDouble()),
            )
        }
    }

    private fun onMessage(m: JSONObject) {
        val from = m.optString("dev")
        if (from.isEmpty() || from == store.deviceId) return
        val to = m.optString("to")
        if (to.isNotEmpty() && to != store.deviceId) return
        val now = SystemClock.elapsedRealtime()
        // the relay hands over what was said while this phone was away: a report of the computer that is long gone is not news
        // (it would make a phantom player of a computer that is shut)
        val said = m.optLong("ts")
        android.util.Log.i("SCDesk", "link <- " + m.optString("t") + " from " + from.take(8) + " age " + (System.currentTimeMillis() - said) / 1000 + "s")
        if (said > 0 && (m.optString("t") == "st" || m.optString("t") == "pres") && System.currentTimeMillis() - said > 45_000) return
        when (m.optString("t")) {
            "pres" -> {
                val old = peers.value[from]
                peers.value = peers.value + (from to (old?.copy(name = m.optString("name"), kind = m.optString("kind", "pc"), seenAt = now)
                    ?: Peer(from, m.optString("name"), m.optString("kind", "pc"), now, null, false, 0, now, 1f)))
                if (old == null) {
                    present()
                    tellState(true)
                }
            }
            "st" -> {
                val t = m.optJSONObject("track")?.let { Track(it.optLong("id"), it.optString("title"), it.optString("artist"), it.optLong("dur"), it.optString("art").takeIf { a -> a.isNotEmpty() && a != "null" }) }
                val old = peers.value[from]
                peers.value = peers.value + (from to Peer(from, old?.name ?: "", old?.kind ?: "pc", now, t, m.optBoolean("playing"), m.optLong("pos"), now, m.optDouble("vol", 1.0).toFloat()))
            }
            "cmd" -> app.engine.run { _ -> command(m) }
            "vpn" -> {
                val list = m.optJSONArray("servers")?.toString()?.let { Tunnel.parse(it) } ?: emptyList()
                app.vpnWaiting.value = false
                if (list.isEmpty()) {
                    app.vpnNote.value = when (m.optString("err")) {
                        "denied" -> "Компьютер отказал: не разрешили на нём."
                        else -> "На компьютере нет подписки с серверами."
                    }
                } else {
                    app.store.vpnServers = m.getJSONArray("servers").toString()
                    app.vpnServers.value = list
                    app.vpnNote.value = "Получено серверов: ${list.size}"
                }
            }
            "rpcr" -> pending[m.optString("id")]?.complete(m.optJSONObject("r") ?: JSONObject().put("ok", false).put("err", "empty answer"))
            "eq" -> m.optJSONObject("eq")?.let { EqState.fromJson(it) }?.let { app.eq.set(it, fromRemote = true) }
            "xfer" -> thread { takeOver(m) }
            "pull" -> app.engine.run { _ -> handOver(from) }
        }
    }

    /** Somebody orders this phone (the computer's «Играть на…»). Main thread. */
    private fun command(m: JSONObject) {
        val e = app.engine
        when (m.optString("op")) {
            "play" -> e.play()
            "pause" -> e.pause()
            "toggle" -> e.toggle()
            "next" -> e.next()
            "prev" -> e.prev()
            "seek" -> e.seek(m.optLong("v"))
            "vol" -> e.volume(m.optDouble("v", 1.0).toFloat())
        }
    }

    /** The music comes here: fetch the tracks with this phone's own login and play them from the same place. */
    private fun takeOver(m: JSONObject) {
        try {
            val arr = m.optJSONArray("ids") ?: JSONArray()
            val ids = (0 until arr.length()).map { arr.getLong(it) }.take(100)
            if (ids.isEmpty()) return
            val tracks = app.api.tracksByIds(ids)
            if (tracks.isEmpty()) return
            if (m.has("vol")) app.engine.volume(m.optDouble("vol", 1.0).toFloat())
            app.engine.playQueue(tracks, 0, m.optLong("pos"), m.optBoolean("playing", true))
        } catch (e: Exception) {
            // the track could not be fetched: the other device goes on playing
        }
    }

    /** The music goes to [to]: the queue from here, the position; this phone stops. Main thread. */
    private fun handOver(to: String) {
        val (ids, pos) = app.engine.queueFromHere(100)
        if (ids.isEmpty()) return
        val playing = app.engine.state.value.playing
        send(JSONObject().put("t", "xfer").put("dev", store.deviceId).put("to", to).put("ids", JSONArray(ids)).put("idx", 0).put("pos", pos).put("playing", playing).put("vol", app.engine.state.value.volume.toDouble()))
        app.engine.pause()
    }

    // ---- what the screens do

    fun command(to: String, op: String, value: Any? = null) {
        send(JSONObject().put("t", "cmd").put("dev", store.deviceId).put("to", to).put("op", op).put("v", value))
    }

    private val pending = java.util.concurrent.ConcurrentHashMap<String, kotlinx.coroutines.CompletableDeferred<JSONObject>>()

    /**
     * Asks the computer for what only it has (friends, shared playlists, rooms, the search inside the lyrics). Gives its
     * answer {ok, data}; throws when it is not there or says no.
     */
    suspend fun rpc(op: String, args: JSONObject = JSONObject(), waitMs: Int = 8_000): JSONObject {
        refreshConnection()
        fun pcNow() = peers.value[store.pcDev] ?: peers.value.values.firstOrNull { it.kind == "pc" }
        // the computer shows up a moment after the link opens
        var pc = pcNow()
        var waited = 0
        while (pc == null && waited < waitMs) { kotlinx.coroutines.delay(400); waited += 400; pc = pcNow() }
        if (pc == null) throw java.io.IOException("Компьютер не в сети")
        val id = java.util.UUID.randomUUID().toString().take(10)
        val d = kotlinx.coroutines.CompletableDeferred<JSONObject>()
        pending[id] = d
        try {
            send(JSONObject().put("t", "rpc").put("dev", store.deviceId).put("to", pc.dev).put("id", id).put("op", op).put("args", args))
            val r = kotlinx.coroutines.withTimeout(30_000) { d.await() }
            android.util.Log.i("SCDesk", "rpc " + op + " -> " + r.toString().take(160))
            if (!r.optBoolean("ok")) throw java.io.IOException(r.optString("err").ifEmpty { "Компьютер отказал" })
            return r
        } finally {
            pending.remove(id)
        }
    }

    /** Asks the computer for the servers of its VPN subscription (it asks its owner first). */
    fun requestVpn() {
        // the link must be up while the answer is awaited
        refreshConnection()
        app.vpnWaiting.value = true
        app.vpnNote.value = "Ждём ответа на компьютере…"
        // no answer for a minute: it is told, the button works again
        Thread {
            Thread.sleep(60_000)
            if (app.vpnWaiting.value) { app.vpnWaiting.value = false; app.vpnNote.value = "Ответа нет. Откройте SC Desk на компьютере и нажмите ещё раз." }
        }.apply { isDaemon = true }.start()
        send(JSONObject().put("t", "vpnreq").put("dev", store.deviceId).put("to", store.pcDev))
    }

    /** The equalizer set on this phone: the other devices take it. */
    fun sendEq(s: EqState) {
        send(JSONObject().put("t", "eq").put("dev", store.deviceId).put("eq", s.toJson()))
    }

    /** "Play on [to]": send this phone's music there. */
    fun pushTo(to: String) = app.engine.run { _ -> handOver(to) }

    /** "Play here": ask [from] to hand its music over to this phone. */
    fun pullFrom(from: String) {
        send(JSONObject().put("t", "pull").put("dev", store.deviceId).put("to", from))
    }
}
