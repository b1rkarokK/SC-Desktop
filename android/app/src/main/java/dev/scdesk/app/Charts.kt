package dev.scdesk.app

import okhttp3.Request
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit

/** A song as a chart lists it (not on SoundCloud yet). */
data class ChartSong(val key: String, val artist: String, val title: String, val durationMs: Long)

/**
 * The real charts, matched onto SoundCloud: the same as the computer's (src-tauri/src/charts.rs). The songs come from
 * Yandex Music (the live chart of a day, the lists of every year since 2001); every song is looked up on SoundCloud
 * strictly (title, artist, length) and a song with no confident match is skipped, never replaced with a wrong track.
 * What was found is remembered for good (a song not found: for a week).
 */
class Charts(private val app: App) {
    companion object {
        /** the same song uploaded several times has one key (the title without brackets and the first artist, letters only) */
        fun songKey(artist: String, title: String): String {
            val clean = title.substringBefore('(').substringBefore('[').substringAfter(" - ", title)
            return (artist.take(0) + clean).lowercase().filter { it.isLetterOrDigit() }
        }
    }

    private val YA = "https://api.music.yandex.net"
    private val memo = ConcurrentHashMap<String, Long>()

    init {
        // what was matched earlier: "key=id;key=id" (0: not found)
        app.store.chartMatches.split(';').forEach { p -> p.split('=').takeIf { it.size == 2 }?.let { memo[it[0]] = it[1].toLongOrNull() ?: 0L } }
    }

    private fun remember() {
        if (memo.size > 3000) memo.keys.take(memo.size - 2500).forEach { memo.remove(it) }
        app.store.chartMatches = memo.entries.joinToString(";") { it.key + "=" + it.value }
    }

    private fun get(url: String): JSONObject = http.newCall(Request.Builder().url(url).header("User-Agent", USER_AGENT).build()).execute().use { r ->
        if (!r.isSuccessful) throw java.io.IOException("Яндекс Музыка ${r.code}")
        JSONObject(r.body?.string() ?: throw java.io.IOException("пусто"))
    }

    private fun song(t: JSONObject): ChartSong? {
        val title = t.optString("title").takeIf { it.isNotEmpty() } ?: return null
        val version = t.optString("version").takeIf { it.isNotEmpty() && it != "null" }
        val artists = t.optJSONArray("artists") ?: return null
        val artist = (0 until artists.length()).map { artists.getJSONObject(it).optString("name") }.filter { it.isNotEmpty() }.joinToString(", ")
        return ChartSong("ya:" + t.opt("id").toString(), artist, if (version != null) "$title ($version)" else title, t.optLong("durationMs"))
    }

    /** The live chart of a day: "russia" or "world". */
    fun live(region: String): List<ChartSong> {
        val tr = get("$YA/landing3/chart/$region").optJSONObject("result")?.optJSONObject("chart")?.optJSONArray("tracks") ?: return emptyList()
        return (0 until tr.length()).mapNotNull { song(tr.getJSONObject(it).optJSONObject("track") ?: return@mapNotNull null) }
    }

    private val years: List<Pair<Int, List<Pair<String, Int>>>> = listOf(
        2025 to listOf("yandexmusic" to 1559), 2024 to listOf("yandexmusic" to 1439), 2023 to listOf("yandexmusic" to 1313),
        2022 to listOf("yandexmusic" to 1150, "yandexmusic" to 1188), 2021 to listOf("yandexmusic" to 1011, "yandexmusic" to 1187, "music-blog" to 2755),
        2020 to listOf("music-blog" to 2020, "music-blog" to 2453), 2019 to listOf("yamusic-research" to 1056, "music-blog" to 2019),
        2018 to listOf("yamusic-research" to 1044, "yamusic-research" to 1045, "music-blog" to 2018),
        2017 to listOf("yamusic-research" to 1009, "music-blog" to 1787, "music-blog" to 2017),
        2016 to listOf("music-blog" to 1481, "music-blog" to 1510, "music-blog" to 2016),
    ) + (2015 downTo 2001).map { it to listOf("music-blog" to it) }

    val yearList: List<Int> get() = years.map { it.first }

    /** The Russian chart of a year: the first of its lists that has songs. */
    fun year(y: Int): List<ChartSong> {
        for ((owner, kind) in years.firstOrNull { it.first == y }?.second ?: emptyList()) {
            val tr = runCatching { get("$YA/users/$owner/playlists/$kind").optJSONObject("result")?.optJSONArray("tracks") }.getOrNull() ?: continue
            val list = (0 until tr.length()).mapNotNull { song(tr.getJSONObject(it).optJSONObject("track") ?: return@mapNotNull null) }
            if (list.isNotEmpty()) return list
        }
        return emptyList()
    }

    // ------------------------------------------------------------ matching

    private fun translit(c: Char): String? = when (c) {
        'а' -> "a"; 'б' -> "b"; 'в' -> "v"; 'г' -> "g"; 'д' -> "d"; 'е', 'ё', 'э' -> "e"; 'ж' -> "zh"; 'з' -> "z"; 'и', 'і' -> "i"
        'й', 'ы' -> "y"; 'к' -> "k"; 'л' -> "l"; 'м' -> "m"; 'н' -> "n"; 'о' -> "o"; 'п' -> "p"; 'р' -> "r"; 'с' -> "s"; 'т' -> "t"
        'у' -> "u"; 'ф' -> "f"; 'х' -> "h"; 'ц' -> "c"; 'ч' -> "ch"; 'ш' -> "sh"; 'щ' -> "sch"; 'ъ', 'ь' -> ""; 'ю' -> "yu"; 'я' -> "ya"
        else -> null
    }

    fun normalize(s: String): String {
        val sb = StringBuilder()
        for (c in s) if (c.isLetterOrDigit()) for (l in c.lowercaseChar().toString()) sb.append(translit(l) ?: l.toString())
        return sb.toString()
    }

    private fun stripBrackets(s: String): String {
        var depth = 0
        val sb = StringBuilder()
        for (c in s) when (c) { '(', '[', '{' -> depth++; ')', ']', '}' -> depth = maxOf(depth - 1, 0); else -> if (depth == 0) sb.append(c) }
        return sb.toString().split(Regex("\\s+")).filter { it.isNotEmpty() }.joinToString(" ")
    }

    private fun stripFeat(s: String): String {
        val l = s.lowercase()
        val cut = listOf(" feat. ", " feat ", " ft. ", " ft ", " prod. ", " prod by ", "+").map { l.indexOf(it) }.filter { it >= 0 }.minOrNull()
        return if (cut != null) s.substring(0, cut).trim() else s.trim()
    }

    private fun firstArtist(s0: String): String {
        val s = stripFeat(s0)
        val l = s.lowercase()
        val cut = listOf(", ", " & ", " x ", " и ", " vs ", "+").map { l.indexOf(it) }.filter { it >= 0 }.minOrNull()
        return if (cut != null) s.substring(0, cut).trim() else s
    }

    private fun clean(s0: String): String {
        val s = s0.split('|', '/').first()
        val words = s.split(Regex("\\s+")).filter { it.isNotEmpty() && !it.startsWith("#") && !it.startsWith("@") }
            .filter { w -> !Regex("[\\p{L}\\p{N}]\\.[\\p{L}\\p{N}]").containsMatchIn(w) }
            .map { w -> w.filter { it.isLetterOrDigit() || it in "'’&-.,!?" } }.filter { it.isNotEmpty() }
        var out = words.joinToString(" ")
        for (junk in listOf("free download", "free dl", "out now", "official video", "official audio", "lyrics", "lyric video")) {
            val i = out.lowercase().indexOf(junk)
            if (i >= 0) out = out.substring(0, i)
        }
        return out.trim().trim('-', ',').trim()
    }

    /** artist and title as a search wants them */
    fun queryFrom(artist0: String, title0: String): Pair<String, String> {
        var artist = artist0
        var title = title0
        for (sep in listOf(" - ", " – ", " — ")) {
            val i = title.indexOf(sep)
            if (i >= 0) { artist = title.substring(0, i); title = title.substring(i + sep.length); break }
        }
        return firstArtist(clean(stripBrackets(artist))) to clean(stripFeat(stripBrackets(title)))
    }

    private val versionMarkers = listOf(
        "remix", "rmx", "edit", "slowed", "sped up", "speed up", "speedup", "nightcore", "8d", "acoustic", "акустик", "cover", "кавер", "live", "концерт",
        "instrumental", "инструментал", "минус", "karaoke", "караоке", "demo", "демо", "mashup", "bootleg", "flip", "rework", "reverb", "bass boost", "bassboost",
        "phonk", "mix)", "version", "версия", "remake", "ремикс", "x2", "hardstyle", "jersey", "drill", "piano", "lofi", "lo-fi", "extended", "vip", "snippet",
        "сниппет", "вокал", "acapella", "a cappella", "acappella", "bbc", "session", "unplugged", "tribute", "blend", "late show", "tiny desk", "sped", "slow",
    )
    private val harmless = setOf(
        "official", "audio", "video", "music", "lyric", "lyrics", "visualizer", "clip", "клип", "премьера", "premiere", "prod", "produced", "by", "feat", "ft",
        "featuring", "with", "explicit", "clean", "hq", "hd", "4k", "remaster", "remastered", "single", "original", "mv", "new", "out", "now", "free", "download",
        "dl", "x", "и", "the", "a", "of", "from", "ost", "саундтрек", "soundtrack",
    ) + (2001..2026).map { it.toString() }

    private fun markers(s: String): List<String> {
        val l = s.lowercase()
        return versionMarkers.filter { m ->
            if (m.length >= 5 && m != "version" && m != "piano") l.contains(m)
            else Regex(Regex.escape(m)).findAll(l).any { r ->
                val before = r.range.first == 0 || !l[r.range.first - 1].isLetterOrDigit()
                val after = r.range.last + 1 >= l.length || !l[r.range.last + 1].isLetterOrDigit()
                before && (after || m.endsWith(")"))
            }
        }
    }

    private fun bracketWords(title: String): List<String> {
        val out = ArrayList<String>()
        var depth = 0
        val cur = StringBuilder()
        for (c in title) {
            when (c) { '(', '[', '{' -> depth++; ')', ']', '}' -> depth = maxOf(depth - 1, 0); else -> if (depth > 0) cur.append(c) }
            if (depth == 0 && cur.isNotEmpty()) { out.addAll(cur.split(Regex("[^\\p{L}\\p{N}]+")).filter { it.isNotEmpty() }.map { it.lowercase() }); cur.clear() }
        }
        return out
    }

    /** The upload is the very recording the chart lists: no version markers it lacks, nothing odd in the brackets. */
    private fun sameRecording(wantTitle: String, wantArtist: String, candTitle: String): Boolean {
        val want = markers(wantTitle)
        if (markers(candTitle).any { it !in want }) return false
        val own = bracketWords(wantTitle) + wantArtist.split(Regex("[^\\p{L}\\p{N}]+")).filter { it.isNotEmpty() }.map { it.lowercase() }
        return bracketWords(candTitle).all { it in harmless || it in own || it.length <= 1 }
    }

    private fun dice(a: String, b: String): Float {
        if (a == b) return 1f
        val pa = a.windowed(2)
        val pb = b.windowed(2).toMutableList()
        if (pa.isEmpty() || pb.isEmpty()) return 0f
        val total = (pa.size + pb.size).toFloat()
        var hit = 0
        for (p in pa) { val i = pb.indexOf(p); if (i >= 0) { pb.removeAt(i); hit++ } }
        return 2f * hit / total
    }

    private class Cand(val id: Long, val title: String, val uploader: String, val artist: String, val dur: Long, val full: Long, val plays: Long, val playable: Boolean, val blocked: Boolean)

    private fun cands(q: String): List<Cand> {
        val items = runCatching { app.api.searchRaw(q) }.getOrDefault(emptyList())
        return items.map { j ->
            val pm = j.optJSONObject("publisher_metadata")
            val up = j.optJSONObject("user")?.optString("username") ?: ""
            val policy = j.optString("policy")
            Cand(
                j.optLong("id"), j.optString("title"), up, pm?.optString("artist")?.trim()?.takeIf { it.isNotEmpty() && it != "null" } ?: up,
                j.optLong("duration"), maxOf(j.optLong("full_duration"), j.optLong("duration")), j.optLong("playback_count"),
                policy != "BLOCK" && policy != "SNIP", policy == "BLOCK" || (j.has("streamable") && !j.optBoolean("streamable", true)),
            )
        }
    }

    /** how sure we are that [t] is [song] (0: no) */
    private fun score(song: ChartSong, t: Cand): Int {
        val (wantArtist, wantTitle) = queryFrom(song.artist, song.title)
        if (!sameRecording(song.title, song.artist, t.title)) return 0
        val wt = normalize(wantTitle)
        val wa = normalize(wantArtist)
        if (wt.isEmpty() || wa.isEmpty()) return 0
        val (titleArtist, scTitle) = queryFrom(t.uploader, t.title)
        if (normalize(scTitle) != wt) return 0
        fun channel(name: String): Boolean { val n = normalize(name); return n == wa || listOf("vevo", "official", "music", "topic", "tv", "channel").any { n == wa + it } }
        val official = channel(t.artist) || channel(t.uploader)
        val named = normalize(titleArtist) == wa
        if (!official && !named) return 0
        val length = if (song.durationMs == 0L) null else Math.abs(t.full - song.durationMs)
        val scoreLen = when {
            length == null -> 0
            length <= 4_000 -> 3
            length <= 12_000 && official -> 1
            length <= 25_000 && official -> -1
            else -> return 0
        }
        if (!official && length == null) return 0
        return 2 + (if (official) 6 else 2) + scoreLen + (if (t.playable) 1 else 0)
    }

    private fun findStrict(song: ChartSong): Cand? {
        val (artist, title) = queryFrom(song.artist, song.title)
        return cands("$artist $title").filter { !it.blocked }.map { score(song, it) to it }.filter { it.first >= 6 }.maxWithOrNull(compareBy({ it.first }, { it.second.plays }))?.second
    }

    private fun findLoose(song: ChartSong): Cand? {
        val (wantArtist, wantTitle) = queryFrom(song.artist, song.title)
        val wa = normalize(wantArtist)
        val wt = normalize(wantTitle)
        if (wa.length < 2 || wt.length < 2) return null
        var best: Pair<Int, Cand>? = null
        for (query in listOf("$wantArtist $wantTitle", wantTitle)) {
            for (t in cands(query).filter { !it.blocked }) {
                val (titleArtist, scTitle) = queryFrom(t.uploader, t.title)
                val st = normalize(scTitle)
                val raw = normalize(t.artist + " " + t.uploader + " " + t.title)
                val sim = maxOf(dice(wt, st), if (raw.contains(wt) && wt.length >= 5) 0.85f else 0f)
                if (sim < 0.8f) continue
                val names = listOf(normalize(t.artist), normalize(t.uploader), normalize(titleArtist))
                val artistOk = raw.contains(wa) || names.any { it.length >= 3 && wa.contains(it) }
                if (!artistOk || !sameRecording(song.title, song.artist, t.title)) continue
                if (song.durationMs > 0 && Math.abs(t.full - song.durationMs) > 20_000) continue
                val sc = (sim * 10).toInt() + (if (names.any { it == wa }) 3 else 0) + (if (t.playable) 1 else 0)
                if (best == null || sc > best.first) best = sc to t
            }
            if (best != null) break
        }
        return best?.second
    }

    /** The id of the SoundCloud track of this song; 0 when it is not there (blocking: the network). */
    fun idOf(song: ChartSong): Long {
        memo[song.key]?.let { return it }
        val t = findStrict(song) ?: findLoose(song)
        memo[song.key] = t?.id ?: 0L
        return t?.id ?: 0L
    }

    /**
     * The tracks of the songs, in the chart's order: [first] of them are looked up at once (for showing), the rest on
     * demand through [more]; four lookups run together.
     */
    suspend fun tracks(songs: List<ChartSong>, limit: Int = songs.size): List<Track> {
        val gate = Semaphore(4)
        val ids = coroutineScope { songs.take(limit).map { s -> async(kotlinx.coroutines.Dispatchers.IO) { gate.withPermit { runCatching { idOf(s) }.getOrDefault(0L) } } }.awaitAll() }
        remember()
        val order = ids.filter { it > 0 }.distinct()
        val found = kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) { app.api.tracksByIds(order) }.associateBy { it.id }
        return order.mapNotNull { found[it] }
    }
}

