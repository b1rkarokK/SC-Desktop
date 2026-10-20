package dev.scdesk.app

import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.Request
import org.json.JSONArray
import org.json.JSONObject

/** What the phone does by itself, without the computer: the translation of lyrics and the search by the words of a song. */
object Independent {
    private fun get(url: String, vararg headers: Pair<String, String>): String =
        http.newCall(Request.Builder().url(url).header("User-Agent", USER_AGENT).apply { headers.forEach { header(it.first, it.second) } }.build()).execute().use { r ->
            if (!r.isSuccessful) throw java.io.IOException("${r.code}")
            r.body?.string() ?: ""
        }

    /** One request of the web translator: the text with its line breaks, translated (blocking). */
    private fun ask(text: String, to: String): String {
        val u = "https://translate.googleapis.com/translate_a/single".toHttpUrl().newBuilder()
            .addQueryParameter("client", "gtx").addQueryParameter("sl", "auto").addQueryParameter("tl", to).addQueryParameter("dt", "t").addQueryParameter("q", text).build()
        val segs = JSONArray(get(u.toString())).optJSONArray(0) ?: return ""
        return (0 until segs.length()).joinToString("") { segs.optJSONArray(it)?.optString(0).orEmpty() }
    }

    /** One translated line for each line (an empty line stays empty); throws when the translator does not answer. */
    fun translate(lines: List<String>, to: String): List<String> {
        val out = MutableList(lines.size) { "" }
        var group = ArrayList<Int>()
        var size = 0
        val groups = ArrayList<List<Int>>()
        lines.forEachIndexed { i, l ->
            if (l.isBlank()) return@forEachIndexed
            if (size + l.length > 1200 && group.isNotEmpty()) { groups.add(group); group = ArrayList(); size = 0 }
            size += l.length + 1
            group.add(i)
        }
        if (group.isNotEmpty()) groups.add(group)
        for (g in groups) {
            val whole = ask(g.joinToString("\n") { lines[it].replace('\n', ' ') }, to)
            val parts = whole.split('\n').map { it.trim() }
            if (parts.size == g.size) g.forEachIndexed { k, i -> out[i] = parts[k] }
            // the line breaks were lost: one by one
            else g.forEach { i -> out[i] = runCatching { ask(lines[i], to).trim() }.getOrDefault("") }
        }
        if (out.none { it.isNotEmpty() }) throw java.io.IOException("перевод недоступен")
        return out
    }

    /** A shuffled list without one artist twice in a row where it can be helped (the first song stays where the shuffle put it). */
    fun smartShuffle(list: List<Track>): List<Track> {
        val out = list.shuffled().toMutableList()
        for (i in 1 until out.size) {
            if (out[i].artist.lowercase() != out[i - 1].artist.lowercase()) continue
            // the nearest later song by somebody else (and not by the artist that follows the place)
            val next = out.getOrNull(i + 1)?.artist?.lowercase()
            val j = (i + 1 until out.size).firstOrNull { out[it].artist.lowercase() != out[i - 1].artist.lowercase() && out[it].artist.lowercase() != next }
                ?: (i + 1 until out.size).firstOrNull { out[it].artist.lowercase() != out[i - 1].artist.lowercase() }
                ?: continue
            val t = out[i]; out[i] = out[j]; out[j] = t
        }
        return out
    }

    private fun songKey(t: Track): String =
        norm(t.title.replace(Regex("\\([^)]*\\)|\\[[^]]*]"), " ").replace(Regex("(?i)\\b(feat|ft)\\.?\\b.*"), "")).replace(Regex("\\s+"), " ").trim() + "|" + norm(t.artist)

    /** The same song liked more than once (the same name and artist, the length within 4 seconds): groups of two or more, the earliest like first. */
    fun duplicates(liked: List<Pair<Track, Long>>): List<List<Track>> =
        liked.sortedBy { it.second }.map { it.first }.groupBy { songKey(it) }.values.flatMap { g ->
            // within a name the lengths must agree: split into runs that are 4 s apart at most
            val runs = ArrayList<MutableList<Track>>()
            g.forEach { t -> runs.firstOrNull { r -> Math.abs(r.first().durationMs - t.durationMs) <= 4_000 }?.add(t) ?: runs.add(mutableListOf(t)) }
            runs.filter { it.size > 1 }
        }

    data class Song(val artist: String, val title: String)

    private fun norm(s: String) = s.lowercase().map { if (it.isLetterOrDigit()) it else ' ' }.joinToString("").trim()

    /** The name of the list and its songs behind a link of Yandex Music or Spotify, or a pasted list «Артист - Трек» per line (blocking). */
    fun readSource(input: String): Pair<String, List<Song>> {
        val text = input.trim()
        if (text.startsWith("http")) {
            val u = java.net.URI(text)
            val host = (u.host ?: "").lowercase()
            val parts = (u.path ?: "").split('/').filter { it.isNotEmpty() && !it.startsWith("intl-") }
            if (host.contains("yandex.") || host.contains("music.ya")) return yandex(parts)
            if (host.contains("spotify.com")) return spotify(parts)
            throw java.io.IOException("Нужна ссылка на плейлист или альбом Яндекс Музыки или Spotify")
        }
        val songs = text.lines().mapNotNull { raw ->
            val l = raw.trim().trimStart { it.isDigit() || it == '.' || it == ')' }.trim()
            val cut = listOf(" - ", " — ", " – ").firstOrNull { l.contains(it) } ?: return@mapNotNull null
            val a = l.substringBefore(cut).trim()
            val t = l.substringAfter(cut).trim()
            if (a.isEmpty() || t.isEmpty()) null else Song(a, t)
        }
        if (songs.isEmpty()) throw java.io.IOException("Вставьте ссылку на плейлист или список строк «Артист - Трек»")
        return "Импорт" to songs
    }

    private fun yandex(parts: List<String>): Pair<String, List<Song>> {
        val api = "https://api.music.yandex.net"
        val notOpen = java.io.IOException("Яндекс Музыка не отдала плейлист: он закрытый или ссылка неверная")
        val (title, tracks) = try {
            when {
                parts.size >= 4 && parts[0] == "users" && parts[2] == "playlists" -> {
                    val r = JSONObject(get("$api/users/${parts[1]}/playlists/${parts[3]}")).getJSONObject("result")
                    r.optString("title") to (0 until r.getJSONArray("tracks").length()).mapNotNull { r.getJSONArray("tracks").getJSONObject(it).optJSONObject("track") }
                }
                parts.size >= 2 && parts[0] == "playlists" -> {
                    val r = JSONObject(get("$api/playlist/${parts[1]}?rich-tracks=true")).getJSONObject("result")
                    r.optString("title") to (0 until r.getJSONArray("tracks").length()).mapNotNull { r.getJSONArray("tracks").getJSONObject(it).optJSONObject("track") }
                }
                parts.size >= 2 && parts[0] == "album" -> {
                    val r = JSONObject(get("$api/albums/${parts[1]}/with-tracks")).getJSONObject("result")
                    val vols = r.getJSONArray("volumes")
                    r.optString("title") to (0 until vols.length()).flatMap { v -> val a = vols.getJSONArray(v); (0 until a.length()).map { a.getJSONObject(it) } }
                }
                else -> throw java.io.IOException("Нужна ссылка на плейлист или альбом Яндекс Музыки")
            }
        } catch (e: java.io.IOException) {
            if (e.message?.startsWith("Нужна") == true) throw e else throw notOpen
        }
        val songs = tracks.mapNotNull { t ->
            val artist = t.optJSONArray("artists")?.optJSONObject(0)?.optString("name").orEmpty()
            val name = t.optString("title")
            if (artist.isEmpty() || name.isEmpty()) null else Song(artist, name)
        }
        if (songs.isEmpty()) throw notOpen
        return title to songs
    }

    private fun spotify(parts: List<String>): Pair<String, List<Song>> {
        val kind = parts.getOrNull(0)?.takeIf { it == "playlist" || it == "album" } ?: throw java.io.IOException("Нужна ссылка на плейлист или альбом Spotify")
        val id = parts.getOrNull(1) ?: throw java.io.IOException("Нужна ссылка на плейлист или альбом Spotify")
        val notOpen = java.io.IOException("Spotify не отдал плейлист: он закрытый или ссылка неверная")
        val html = get("https://open.spotify.com/embed/$kind/$id")
        val json = html.substringAfter("<script id=\"__NEXT_DATA__\" type=\"application/json\">", "").substringBefore("</script>")
        if (json.isEmpty()) throw notOpen
        val e = JSONObject(json).optJSONObject("props")?.optJSONObject("pageProps")?.optJSONObject("state")?.optJSONObject("data")?.optJSONObject("entity") ?: throw notOpen
        val list = e.optJSONArray("trackList") ?: throw notOpen
        val songs = (0 until list.length()).mapNotNull { i ->
            val t = list.getJSONObject(i)
            val artist = t.optString("subtitle").replace('\u00a0', ' ')
            val name = t.optString("title")
            if (artist.isEmpty() || name.isEmpty()) null else Song(artist, name)
        }
        if (songs.isEmpty()) throw notOpen
        return e.optString("name").ifEmpty { e.optString("title").ifEmpty { "Spotify" } } to songs
    }

    /** Finds each song on SoundCloud (the first result that holds the words of the title) and makes the playlist. Gives how many were found. Blocking. */
    fun importPlaylist(app: App, input: String, title: String, progress: (Int, Int) -> Unit): Triple<String, Int, Int> {
        val (name, songs) = readSource(input)
        val ids = ArrayList<Long>()
        songs.forEachIndexed { i, s ->
            progress(i, songs.size)
            val words = norm(s.title).split(' ').filter { it.length >= 3 }
            val hit = runCatching { app.api.search("${s.artist} ${s.title}") }.getOrNull()?.firstOrNull { c ->
                val hay = norm(c.title + " " + c.artist)
                words.isEmpty() || words.all { hay.contains(it) }
            }
            if (hit != null && !ids.contains(hit.id)) ids.add(hit.id)
        }
        if (ids.isEmpty()) throw java.io.IOException("Ни одной песни не нашлось на SoundCloud")
        val finalTitle = title.ifBlank { name }
        val chunks = ids.chunked(500)
        chunks.forEachIndexed { i, chunk -> app.api.createPlaylistWith(if (chunks.size > 1) "$finalTitle (${i + 1}/${chunks.size})" else finalTitle, chunk) }
        return Triple(finalTitle, ids.size, songs.size)
    }

    /** Songs by the words in their texts: NetEase searches inside the lyrics, then each song is looked for on SoundCloud (blocking). */
    fun songsByWords(app: App, words: String): List<Track> {
        val u = "https://music.163.com/api/search/get".toHttpUrl().newBuilder().addQueryParameter("s", words).addQueryParameter("type", "1006").addQueryParameter("limit", "8").build()
        val songs = JSONObject(get(u.toString(), "Referer" to "https://music.163.com/")).optJSONObject("result")?.optJSONArray("songs") ?: return emptyList()
        val found = ArrayList<Track>()
        for (i in 0 until minOf(songs.length(), 6)) {
            val s = songs.getJSONObject(i)
            val artist = s.optJSONArray("artists")?.optJSONObject(0)?.optString("name").orEmpty()
            val title = s.optString("name")
            if (title.isEmpty()) continue
            val hit = runCatching { app.api.search("$artist $title").firstOrNull() }.getOrNull() ?: continue
            if (found.none { it.id == hit.id }) found.add(hit)
        }
        return found
    }
}
