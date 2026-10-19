package dev.scdesk.app

import android.util.LruCache
import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject
import java.io.IOException

const val USER_AGENT = "Mozilla/5.0 (Linux; Android 13) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36"

/**
 * The public client_id of the SoundCloud web site, as it was when this build was made. The site (soundcloud.com) is
 * closed at some providers, so the app cannot always read the current one from it; then this one is used until the
 * computer gives a fresh one (the pairing) or the site opens. It is not a secret: every visitor of the site has it.
 */
const val FALLBACK_CLIENT_ID = "tHVbSbDP4TaoMbPppngv6Tdry83vNbMQ"

data class Track(val id: Long, val title: String, val artist: String, val durationMs: Long, val artwork: String?, val artistId: Long = 0, val words: String = "")

/** A card of the home page: a mix of SoundCloud ("mix", by [urn]) or a playlist (by [id]). */
data class Card(val kind: String, val id: Long, val urn: String?, val title: String, val subtitle: String, val artwork: String?)

data class Shelf(val title: String, val cards: List<Card>)

/** An artist (from the followings). */
data class Artist(val id: Long, val name: String, val avatar: String?)

/** SoundCloud's own API (api-v2), the same calls the computer makes; the login came from the QR pairing. */
class ScApi(private val store: Store) {
    private val base = "https://api-v2.soundcloud.com"

    /** the whole JSON of recent tracks: the stream address is made from its `media` */
    private val raw = LruCache<Long, JSONObject>(400)

    /** The public client_id, read from the site when there is none yet (a guest has no other). */
    private var discoveryTried = false

    @Synchronized
    private fun clientId(): String {
        if (store.clientId.isEmpty()) {
            // the site once (it may be closed here: then the one built in), not on every request
            store.clientId = try {
                if (discoveryTried) FALLBACK_CLIENT_ID else { discoveryTried = true; discoverClientId() }
            } catch (e: Exception) {
                FALLBACK_CLIENT_ID
            }
        }
        return store.clientId
    }

    private fun url(path: String, vararg query: Pair<String, String>): HttpUrl {
        val b = (base + path).toHttpUrl().newBuilder()
        for ((k, v) in query) b.addQueryParameter(k, v)
        b.addQueryParameter("client_id", clientId())
        b.addQueryParameter("app_locale", "en")
        return b.build()
    }

    /** answers kept for a few minutes: the tabs are built again at every switch, and the net is slow, so they must not ask again */
    private val seen = java.util.concurrent.ConcurrentHashMap<String, Pair<String, Long>>()

    /** where the answers are kept between runs (the cache folder), set by the app */
    @Volatile var dir: java.io.File? = null
    private val staleOk = ThreadLocal<Boolean>()

    /** Runs [block] so that an answer kept on the disk, even old, is taken at once (for showing a list before the fresh one comes). */
    fun <T> stale(block: () -> T): T {
        staleOk.set(true)
        try { return block() } finally { staleOk.set(false) }
    }

    private fun diskFile(key: String): java.io.File? {
        val d = dir ?: return null
        val h = java.security.MessageDigest.getInstance("SHA-1").digest((key + "|" + store.userId).toByteArray()).joinToString("") { "%02x".format(it) }
        return java.io.File(d, "api-$h")
    }

    private fun getText(url: HttpUrl, token: String = store.oauthToken): String {
        // the signed stream addresses are good for minutes only and are kept by hlsUrl itself: not here
        if (url.encodedPath.startsWith("/media/")) return fetchText(url, token)
        val key = url.toString() + "|" + token.length
        seen[key]?.let { (text, at) -> if (System.currentTimeMillis() - at < 3 * 60_000) return text }
        val file = diskFile(key)
        if (staleOk.get() == true && file != null && file.exists() && System.currentTimeMillis() - file.lastModified() < 24 * 3600_000L) {
            runCatching { file.readText() }.getOrNull()?.let { return it }
        }
        val text = fetchText(url, token)
        if (seen.size > 80) seen.clear()
        seen[key] = text to System.currentTimeMillis()
        if (file != null) runCatching {
            file.parentFile?.mkdirs()
            file.writeText(text)
            val all = file.parentFile?.listFiles()
            if (all != null && all.size > 120) all.sortedBy { it.lastModified() }.take(all.size - 100).forEach { it.delete() }
        }
        return text
    }

    /** A write to SoundCloud straight from the phone. Throws when SoundCloud refuses (the computer is then asked to do it). */
    fun write(method: String, path: String, json: String? = null) {
        val body = (json ?: "").toRequestBody("application/json".toMediaType())
        val req = Request.Builder().url(url(path)).method(method, if (method == "DELETE" && json == null) null else body)
            .header("User-Agent", USER_AGENT).header("Accept", "application/json").header("Authorization", "OAuth ${store.oauthToken}").build()
        http.newCall(req).execute().use { r ->
            android.util.Log.i("SCDesk", "$method $path -> ${r.code}")
            if (!r.isSuccessful) throw java.io.IOException("SoundCloud ${r.code}")
        }
        forget()
    }

    fun likeTrack(id: Long, on: Boolean) = write(if (on) "PUT" else "DELETE", "/users/${store.userId}/track_likes/$id")
    fun followUser(id: Long, on: Boolean) = write(if (on) "PUT" else "DELETE", "/me/followings/$id")
    fun deletePlaylist(id: Long) = write("DELETE", "/playlists/$id")

    /** The user's own playlists: number, name, how many tracks. */
    fun ownPlaylists(): List<Triple<Long, String, Int>> {
        val j = JSONObject(fetchText(url("/users/${store.userId}/playlists_without_albums", "limit" to "100"), store.oauthToken))
        val a = j.optJSONArray("collection") ?: return emptyList()
        return (0 until a.length()).map { a.getJSONObject(it) }.map { Triple(it.getLong("id"), it.optString("title"), it.optInt("track_count")) }
    }

    private fun playlistTrackIds(id: Long): List<Long> {
        val j = JSONObject(fetchText(url("/playlists/$id"), store.oauthToken))
        val a = j.optJSONArray("tracks") ?: return emptyList()
        return (0 until a.length()).map { a.getJSONObject(it).getLong("id") }
    }

    private fun tracksBody(ids: List<Long>) = JSONObject().put("playlist", JSONObject().put("tracks", JSONArray(ids))).toString()

    fun addToPlaylist(playlist: Long, track: Long) {
        val ids = playlistTrackIds(playlist)
        if (ids.contains(track)) throw java.io.IOException("Этот трек уже есть в плейлисте")
        write("PUT", "/playlists/$playlist", tracksBody(ids + track))
    }

    fun removeFromPlaylist(playlist: Long, track: Long) = write("PUT", "/playlists/$playlist", tracksBody(playlistTrackIds(playlist).filter { it != track }))

    fun createPlaylist(title: String, firstTrack: Long?) =
        write("POST", "/playlists", JSONObject().put("playlist", JSONObject().put("title", title).put("sharing", "private").put("tracks", JSONArray(listOfNotNull(firstTrack)))).toString())

    /** forgets what was kept (a new account, or the user pulls to refresh) */
    fun forget() = seen.clear()

    private fun fetchText(url: HttpUrl, token: String): String {
        val req = Request.Builder().url(url).header("User-Agent", USER_AGENT).header("Accept", "application/json").apply {
            if (token.isNotEmpty()) header("Authorization", "OAuth $token")
        }.build()
        http.newCall(req).execute().use { r ->
            android.util.Log.i("SCDesk", "GET ${url.encodedPath} -> ${r.code}")
            if (r.code == 401 && store.oauthToken.isEmpty() && token.isEmpty()) {
                // the public client_id was turned: read the new one and ask once more
                store.clientId = ""
            }
            if (!r.isSuccessful) throw IOException("SoundCloud ${r.code}")
            return r.body?.string() ?: throw IOException("empty answer")
        }
    }

    private fun toTrack(j: JSONObject): Track? {
        val id = j.optLong("id", 0)
        if (id == 0L) return null
        raw.put(id, j)
        val user = j.optJSONObject("user")
        val art = j.optString("artwork_url").takeIf { it.isNotEmpty() && it != "null" } ?: user?.optString("avatar_url")?.takeIf { it.isNotEmpty() && it != "null" }
        val dur = maxOf(j.optLong("duration"), j.optLong("full_duration"))
        val words = listOf(j.optString("genre"), j.optString("tag_list"), j.optString("title")).filter { it != "null" }.joinToString(" ")
        return Track(id, j.optString("title"), user?.optString("username") ?: "", dur, art?.replace("-large.", "-t300x300."), user?.optLong("id") ?: 0, words)
    }

    /** The account of a login token: its id and name (the sign-in here has only the token). */
    /** The address of the track's page on SoundCloud (blocking: the network); null when it has none. */
    fun permalink(id: Long): String? = runCatching { JSONObject(getText(url("/tracks/$id"))).optString("permalink_url").takeIf { it.startsWith("https://") } }.getOrNull()

    fun me(): Pair<Long, String> {
        val j = JSONObject(getText(url("/me")))
        return j.getLong("id") to j.optString("username")
    }

    companion object {
        /** The public client_id is in the scripts of soundcloud.com; read from there like the computer does. */
        fun discoverClientId(): String {
            // the site may hang (a provider that blocks it): the answer is not waited for longer than this
            val quick = http.newBuilder().callTimeout(8, java.util.concurrent.TimeUnit.SECONDS).build()
            fun fetch(u: String): String = quick.newCall(Request.Builder().url(u).header("User-Agent", USER_AGENT).build()).execute().use { it.body?.string() ?: "" }
            val html = fetch("https://soundcloud.com/")
            val scripts = Regex("src=\"(https://a-v2\\.sndcdn\\.com/assets/[^\"]+\\.js)\"").findAll(html).map { it.groupValues[1] }.toList().asReversed()
            for (s in scripts.take(8)) {
                Regex("client_id:\"([A-Za-z0-9]{20,40})\"").find(fetch(s))?.let { return it.groupValues[1] }
            }
            android.util.Log.w("SCDesk", "client_id not found in ${scripts.size} scripts")
            throw IOException("client_id not found")
        }
    }

    /** The liked tracks, newest first: a page and the address of the next one. */
    fun likes(next: String? = null, limit: Int = 50): Pair<List<Track>, String?> {
        val u = if (next != null) next.toHttpUrl().newBuilder().apply {
            if (next.toHttpUrl().queryParameter("client_id") == null) addQueryParameter("client_id", store.clientId)
        }.build() else url("/users/${store.userId}/track_likes", "limit" to limit.toString(), "linked_partitioning" to "1")
        val j = JSONObject(getText(u))
        val items = j.optJSONArray("collection") ?: JSONArray()
        val out = ArrayList<Track>()
        for (i in 0 until items.length()) items.getJSONObject(i).optJSONObject("track")?.let { toTrack(it)?.let(out::add) }
        return out to j.optString("next_href").takeIf { it.isNotEmpty() && it != "null" && out.isNotEmpty() }
    }

    /** The whole JSON of the tracks a search finds (for matching a song to its upload). */
    fun searchRaw(q: String): List<JSONObject> {
        val items = JSONObject(getText(url("/search/tracks", "q" to q, "limit" to "30"))).optJSONArray("collection") ?: JSONArray()
        return (0 until items.length()).map { items.getJSONObject(it) }.onEach { j -> j.optLong("id").takeIf { it != 0L }?.let { raw.put(it, j) } }
    }

    /** The likes with the day they were made (ms), the newest first: the picks «Забытые лайки» and «Год назад» are made of them. */
    fun likesDated(max: Int = 200): List<Pair<Track, Long>> {
        val out = ArrayList<Pair<Track, Long>>()
        var next: String? = null
        while (out.size < max) {
            val u = if (next != null) next.toHttpUrl().newBuilder().apply { if (next!!.toHttpUrl().queryParameter("client_id") == null) addQueryParameter("client_id", store.clientId) }.build()
            else url("/users/${store.userId}/track_likes", "limit" to "50", "linked_partitioning" to "1")
            val j = JSONObject(getText(u))
            val items = j.optJSONArray("collection") ?: break
            for (i in 0 until items.length()) {
                val it = items.getJSONObject(i)
                val t = it.optJSONObject("track")?.let { toTrack(it) } ?: continue
                val at = runCatching { java.time.Instant.parse(it.optString("created_at")).toEpochMilli() }.getOrDefault(0L)
                out.add(t to at)
            }
            next = j.optString("next_href").takeIf { it.isNotEmpty() && it != "null" } ?: break
        }
        return out
    }

    /** how many times a track was played on SoundCloud (0 when not known) */
    fun plays(id: Long): Long = raw.get(id)?.optLong("playback_count") ?: 0L

    /** Tracks by a search: public, works without the sign-in. */
    fun search(q: String, offset: Int = 0): List<Track> {
        val items = JSONObject(getText(url("/search/tracks", "q" to q, "limit" to "30", "offset" to offset.toString()))).optJSONArray("collection") ?: JSONArray()
        return (0 until items.length()).mapNotNull { toTrack(items.getJSONObject(it)) }
    }

    fun searchUsers(q: String): List<Artist> {
        val items = JSONObject(getText(url("/search/users", "q" to q, "limit" to "10"))).optJSONArray("collection") ?: JSONArray()
        return (0 until items.length()).map { items.getJSONObject(it) }.map { Artist(it.optLong("id"), it.optString("username"), it.optString("avatar_url").takeIf { a -> a.isNotEmpty() && a != "null" }?.replace("-large.", "-t200x200.")) }
    }

    fun searchPlaylists(q: String): List<Card> {
        val items = JSONObject(getText(url("/search/playlists", "q" to q, "limit" to "10"))).optJSONArray("collection") ?: JSONArray()
        return (0 until items.length()).map { items.getJSONObject(it) }.map { Card("playlist", it.optLong("id"), null, it.optString("title"), it.optJSONObject("user")?.optString("username") ?: "", art(it)) }
    }

    /** The covers of the first three tracks of a live chart (for the tile of a category). */
    fun mixCovers(urn: String): List<String?> {
        val arr = JSONObject(getText(url("/system-playlists/$urn"))).optJSONArray("tracks") ?: return emptyList()
        val ids = (0 until minOf(arr.length(), 3)).mapNotNull { arr.getJSONObject(it).optLong("id").takeIf { id -> id != 0L } }
        return tracksByIds(ids).map { it.artwork }
    }

    /**
     * A theme as tracks, the way the computer does it (home.rs category_tracks): the playlists the searches find, the five most
     * liked of them, and the tracks they agree on come first (then by plays), one upload per song.
     */
    fun byQueries(queries: List<String>): List<Track> {
        val found = LinkedHashMap<Long, JSONObject>()
        for (q in queries.take(4)) {
            val items = runCatching { JSONObject(getText(url("/search/playlists", "q" to q, "limit" to "30"))).optJSONArray("collection") }.getOrNull() ?: continue
            for (i in 0 until items.length()) items.getJSONObject(i).let { if (it.optInt("track_count") >= 10) found.putIfAbsent(it.optLong("id"), it) }
        }
        val top = found.values.sortedByDescending { it.optLong("likes_count") }.take(5)
        val count = HashMap<Long, Int>()
        val order = ArrayList<Long>()
        for (p in top) {
            val arr = runCatching { JSONObject(getText(url("/playlists/${p.optLong("id")}", "representation" to "full"))).optJSONArray("tracks") }.getOrNull() ?: continue
            for (k in 0 until minOf(arr.length(), 80)) {
                val id = arr.getJSONObject(k).optLong("id").takeIf { it != 0L } ?: continue
                if (count.merge(id, 1, Int::plus) == 1) order.add(id)
            }
        }
        if (order.isEmpty()) return bySearch(queries)
        order.sortByDescending { count[it] }
        val tracks = tracksByIds(order.take(150)).sortedWith(compareByDescending<Track> { count[it.id] ?: 0 }.thenByDescending { plays(it.id) })
        val seen = HashSet<String>()
        return tracks.filter { seen.add(Charts.songKey(it.artist, it.title)) }.take(100)
    }

    /** The tracks of a search-made category: the searches one after another, mixed in turn, no repeats. */
    private fun bySearch(queries: List<String>): List<Track> {
        val lists = queries.take(3).map { q -> runCatching { search(q) }.getOrDefault(emptyList()) }
        val out = LinkedHashMap<Long, Track>()
        for (i in 0 until (lists.maxOfOrNull { it.size } ?: 0)) for (l in lists) l.getOrNull(i)?.let { out.putIfAbsent(it.id, it) }
        return out.values.toList()
    }

    // ---- home

    private fun shelfTitle(urn: String, fallback: String): String = when (urn.split(':').getOrNull(2) ?: "") {
        "personalized-tracks" -> "Больше того, что вам нравится"
        "your-moods" -> "Ваши миксы"
        "made-for-you" -> "Сделано для вас"
        "personalised-curated-global", "curated" -> "Подборки SoundCloud"
        "personalized-albums" -> "Альбомы для вас"
        "artist-stations" -> "Станции артистов"
        "trending-by-genre-playlists", "trending-by-genre" -> "В тренде по жанрам"
        "buzzing" -> "Стоит послушать"
        else -> if ((urn.split(':').getOrNull(2) ?: "").startsWith("liked-by")) "Любимое у других" else fallback
    }

    private fun cardTitle(t: String): String = when {
        t.startsWith("Your Mix ") -> "Ваш микс " + t.removePrefix("Your Mix ")
        t.startsWith("Related tracks: ") -> "Похоже на «" + t.removePrefix("Related tracks: ") + "»"
        t.endsWith("'s Picks") -> "Выбор " + t.removeSuffix("'s Picks")
        else -> t
    }

    private fun art(v: JSONObject): String? = (v.optString("calculated_artwork_url").takeIf { it.isNotEmpty() && it != "null" } ?: v.optString("artwork_url").takeIf { it.isNotEmpty() && it != "null" })?.replace("-large.", "-t300x300.")

    /** The shelves of the home page (the mixes SoundCloud makes for this account). */
    fun home(): List<Shelf> {
        val coll = JSONObject(getText(url("/mixed-selections", "limit" to "10"))).optJSONArray("collection") ?: return emptyList()
        val out = ArrayList<Shelf>()
        for (i in 0 until coll.length()) {
            val s = coll.getJSONObject(i)
            val urn = s.optString("urn")
            if (urn.contains(":recently-played")) continue
            val items = s.optJSONObject("items")?.optJSONArray("collection") ?: continue
            val cards = ArrayList<Card>()
            for (k in 0 until items.length()) {
                val v = items.getJSONObject(k)
                val title = cardTitle(v.optString("title").ifEmpty { v.optString("short_title") })
                when (v.optString("kind")) {
                    "system-playlist" -> cards.add(Card("mix", 0, v.optString("urn"), title, v.optString("short_description").takeIf { it != "null" }?.take(60) ?: "", art(v)))
                    "playlist" -> cards.add(Card("playlist", v.optLong("id"), null, title, v.optJSONObject("user")?.optString("username") ?: "", art(v)))
                }
            }
            if (cards.isNotEmpty()) out.add(Shelf(shelfTitle(urn, s.optString("title")), cards))
        }
        return out
    }

    /** The tracks of a mix of SoundCloud. */
    fun mixTracks(urn: String): List<Track> {
        val arr = JSONObject(getText(url("/system-playlists/$urn"))).optJSONArray("tracks") ?: return emptyList()
        return tracksByIds((0 until arr.length()).mapNotNull { arr.getJSONObject(it).optLong("id").takeIf { id -> id != 0L } })
    }

    /** The tracks of a playlist. */
    fun playlistTracks(id: Long): List<Track> {
        val arr = JSONObject(getText(url("/playlists/$id", "representation" to "full"))).optJSONArray("tracks") ?: return emptyList()
        return tracksByIds((0 until arr.length()).mapNotNull { arr.getJSONObject(it).optLong("id").takeIf { i -> i != 0L } })
    }

    /** New tracks of the artists followed. */
    fun fromFollowings(): List<Track> {
        val items = JSONObject(getText(url("/stream", "limit" to "40"))).optJSONArray("collection") ?: return emptyList()
        val out = ArrayList<Track>()
        for (i in 0 until items.length()) {
            val it = items.getJSONObject(i)
            if (it.optString("type").startsWith("track")) it.optJSONObject("track")?.let { t -> toTrack(t)?.let(out::add) }
        }
        return out.distinctBy { it.id }
    }

    /** The playlists the account liked (and made). */
    fun likedPlaylists(): List<Card> {
        val items = JSONObject(getText(url("/users/${store.userId}/playlist_likes", "limit" to "50"))).optJSONArray("collection") ?: return emptyList()
        return (0 until items.length()).mapNotNull { items.getJSONObject(it).optJSONObject("playlist") }.map {
            Card("playlist", it.optLong("id"), null, it.optString("title"), it.optJSONObject("user")?.optString("username") ?: "", art(it))
        }
    }

    /** The artists followed. */
    fun followings(): List<Artist> {
        val items = JSONObject(getText(url("/users/${store.userId}/followings", "limit" to "100"))).optJSONArray("collection") ?: return emptyList()
        return (0 until items.length()).map { items.getJSONObject(it) }.map { Artist(it.optLong("id"), it.optString("username"), it.optString("avatar_url").takeIf { a -> a.isNotEmpty() && a != "null" }?.replace("-large.", "-t200x200.")) }
    }

    /** The uploads of an artist. */
    fun artistTracks(id: Long): List<Track> {
        val items = JSONObject(getText(url("/users/$id/tracks", "limit" to "50"))).optJSONArray("collection") ?: return emptyList()
        return (0 until items.length()).mapNotNull { toTrack(items.getJSONObject(it)) }
    }

    /** What SoundCloud finds related to a track: the candidates of the wave. */
    fun related(trackId: Long): List<Track> {
        val items = JSONObject(getText(url("/tracks/$trackId/related", "limit" to "50"))).optJSONArray("collection") ?: return emptyList()
        return (0 until items.length()).mapNotNull { toTrack(items.getJSONObject(it)) }
    }

    /** The tracks by their ids, in the order of the ids (fifty at a time). */
    fun tracksByIds(ids: List<Long>): List<Track> {
        val found = HashMap<Long, Track>()
        for (chunk in ids.chunked(50)) {
            val arr = JSONArray(getText(url("/tracks", "ids" to chunk.joinToString(","))))
            for (i in 0 until arr.length()) toTrack(arr.getJSONObject(i))?.let { found[it.id] = it }
        }
        return ids.mapNotNull { found[it] }
    }

    private val streams = java.util.concurrent.ConcurrentHashMap<Long, Pair<String, Long>>()

    /** Looks the stream addresses of the next tracks up beforehand, so that a skip does not wait for them (blocking). */
    fun warm(ids: List<Long>) {
        for (id in ids) runCatching { hlsUrl(id) }
    }

    /** The HLS playlist address of a track, signed, good for a few minutes. */
    fun hlsUrl(id: Long): String {
        streams[id]?.let { (u, at) -> if (System.currentTimeMillis() - at < 4 * 60_000) return u }
        val u = resolveHls(id)
        if (streams.size > 40) streams.clear()
        streams[id] = u to System.currentTimeMillis()
        return u
    }

    private fun resolveHls(id: Long): String {
        val j = raw.get(id) ?: JSONArray(getText(url("/tracks", "ids" to id.toString()))).getJSONObject(0).also { raw.put(id, it) }
        val tcs = j.optJSONObject("media")?.optJSONArray("transcodings") ?: throw IOException("no media")
        // AAC first (smaller), then MP3; a 30-second preview only when nothing else is there
        data class Tc(val url: String, val rank: Int)
        val list = ArrayList<Tc>()
        for (i in 0 until tcs.length()) {
            val t = tcs.getJSONObject(i)
            val f = t.getJSONObject("format")
            if (f.optString("protocol") != "hls") continue
            val mime = f.optString("mime_type")
            val rank = when {
                mime.startsWith("audio/mp4") -> 0
                mime.startsWith("audio/mpeg") -> 1
                else -> continue
            } + if (t.optBoolean("snipped")) 10 else 0
            list.add(Tc(t.getString("url"), rank))
        }
        if (list.isEmpty()) throw IOException("no stream for this track")
        list.sortBy { it.rank }
        val auth = j.optString("track_authorization")
        var last: Exception? = null
        for (tc in list) {
            for (token in listOf(store.oauthToken, store.streamToken).filter { it.isNotEmpty() }.ifEmpty { listOf("") }) {
                try {
                    val u = tc.url.toHttpUrl().newBuilder().apply {
                        if (tc.url.toHttpUrl().queryParameter("client_id") == null) addQueryParameter("client_id", store.clientId)
                        if (auth.isNotEmpty()) addQueryParameter("track_authorization", auth)
                    }.build()
                    return JSONObject(getText(u, token)).getString("url")
                } catch (e: Exception) {
                    last = e
                }
            }
        }
        throw IOException("stream not resolved", last)
    }
}
