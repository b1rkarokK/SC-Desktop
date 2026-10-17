package dev.scdesk.app

import kotlin.math.pow
import kotlin.random.Random

/** The moods of the wave: the same words as on the computer (VibeWords.kt is made from src-tauri/src/wave_vibe.rs). */
object Vibes {
    val all = listOf(
        "any" to "Любое", "calm" to "Спокойное", "energetic" to "Энергичное", "sad" to "Грустное", "happy" to "Весёлое",
        "aggressive" to "Агрессивное", "romantic" to "Романтичное", "party" to "Для вечеринки", "focus" to "Для работы", "night" to "Ночное",
    )

    fun tokens(text: String): List<String> = text.lowercase().split(Regex("[^\\p{L}\\p{N}-]+")).filter { it.isNotEmpty() }

    private fun hits(list: List<String>, toks: List<String>): Int = list.count { w ->
        if (w.endsWith("*")) w.dropLast(1).let { stem -> toks.any { it.startsWith(stem) } } else toks.any { it == w }
    }

    /** How much the chosen mood multiplies the chance of a track with these words (1 when no mood is chosen). */
    fun factor(vibe: String, words: String): Double {
        val w = VIBE_WORDS[vibe] ?: return 1.0
        val toks = tokens(words)
        val yes = hits(w.yes, toks)
        val no = hits(w.no, toks)
        return when {
            yes == 0 && no == 0 -> 0.5
            yes == 0 -> 0.12
            no == 0 -> minOf(2.8 + 0.5 * (minOf(yes, 3) - 1), 3.8)
            yes > no -> 1.4
            else -> 0.3
        }
    }
}

/**
 * «Моя волна» of the phone: the likes give the seeds, SoundCloud's own «related» of each seed the candidates; an
 * artist that played lately sits out, no more than two songs of one artist in a batch, the chosen mood leads. The
 * computer has the learned taste model on top of this (src-tauri/src/wave.rs); the phone has the simple part of it.
 */
class Wave(private val app: App) {
    @Volatile var active = false
    @Volatile var vibe = "any"

    private var likes: List<Track> = emptyList()
    private val played = LinkedHashSet<Long>()
    private val recentArtists = ArrayDeque<Long>()
    private val rng = Random.Default

    /** why a track is in the wave: «похоже на «…»» */
    val reasons = HashMap<Long, String>()

    // the settings, as on the computer: the newness (0 as usual, 1 more new, 2 only the known), the language (0 any, 1 Russian, 2 foreign),
    // without the liked, without the heard; the artists and the tracks the user does not want. All of it is kept between runs
    private val nov = androidx.compose.runtime.mutableIntStateOf(0)
    private val lang = androidx.compose.runtime.mutableIntStateOf(0)
    private val noLikedS = androidx.compose.runtime.mutableStateOf(true)
    private val noHeardS = androidx.compose.runtime.mutableStateOf(false)
    val bannedArtists = androidx.compose.runtime.mutableStateMapOf<Long, String>()
    val dislikedList = androidx.compose.runtime.mutableStateListOf<Track>()

    var novelty: Int get() = nov.intValue; set(v) { nov.intValue = v; save() }
    var language: Int get() = lang.intValue; set(v) { lang.intValue = v; save() }
    var noLiked: Boolean get() = noLikedS.value; set(v) { noLikedS.value = v; save() }
    var noHeard: Boolean get() = noHeardS.value; set(v) { noHeardS.value = v; save() }

    init {
        try {
            val j = org.json.JSONObject(app.store.waveJson.ifEmpty { "{}" })
            nov.intValue = j.optInt("novelty"); lang.intValue = j.optInt("language"); noLikedS.value = j.optBoolean("noLiked", true); noHeardS.value = j.optBoolean("noHeard")
            j.optJSONArray("bans")?.let { a -> for (i in 0 until a.length()) a.getJSONObject(i).let { bannedArtists[it.getLong("id")] = it.optString("n") } }
            j.optJSONArray("dis")?.let { a -> for (i in 0 until a.length()) a.getJSONObject(i).let { dislikedList.add(Track(it.getLong("i"), it.optString("t"), it.optString("a"), 0, null)) } }
        } catch (e: Exception) { /* the defaults */ }
    }

    private fun save() {
        val j = org.json.JSONObject().put("novelty", nov.intValue).put("language", lang.intValue).put("noLiked", noLikedS.value).put("noHeard", noHeardS.value)
        j.put("bans", org.json.JSONArray().also { a -> bannedArtists.forEach { (id, n) -> a.put(org.json.JSONObject().put("id", id).put("n", n)) } })
        j.put("dis", org.json.JSONArray().also { a -> dislikedList.takeLast(300).forEach { a.put(org.json.JSONObject().put("i", it.id).put("t", it.title).put("a", it.artist)) } })
        app.store.waveJson = j.toString()
    }

    /** the track does not come back into the wave */
    fun dislike(t: Track) { if (dislikedList.none { it.id == t.id }) dislikedList.add(t); save() }
    fun undislike(id: Long) { dislikedList.removeAll { it.id == id }; save() }
    fun banArtist(t: Track) { if (t.artistId != 0L) { bannedArtists[t.artistId] = t.artist; save() } }
    fun unbanArtist(id: Long) { bannedArtists.remove(id); save() }
    fun resetBans() { bannedArtists.clear(); dislikedList.clear(); save() }

    private fun ensureLikes() {
        if (likes.isNotEmpty()) return
        val all = ArrayList<Track>()
        var next: String? = null
        repeat(3) {
            val (page, n) = app.api.likes(next)
            all += page
            next = n
            if (n == null) return@repeat
        }
        likes = all
    }

    fun reset() {
        played.clear()
        recentArtists.clear()
        reasons.clear()
        likes = emptyList()
    }

    /** A batch of tracks for the queue (blocking: the network). */
    fun batch(n: Int = 25): List<Track> {
        ensureLikes()
        val liked = likes
        if (liked.isEmpty()) return emptyList()
        val likedIds = liked.map { it.id }.toHashSet()

        // seeds: from the newer likes, those of the mood first
        val seeds = weightedPick(liked.withIndex().toList(), 5) { (i, t) -> (1.0 / (1.0 + i / 200.0)) * Vibes.factor(vibe, t.words).coerceAtLeast(0.3) }.map { it.value }
        val hits = HashMap<Long, Pair<Track, Int>>()
        val via = HashMap<Long, String>()
        for (seed in seeds) {
            val rel = try { app.api.related(seed.id) } catch (e: Exception) { emptyList() }
            for (t in rel) {
                hits[t.id] = t to ((hits[t.id]?.second ?: 0) + 1)
                via.putIfAbsent(t.id, "похоже на «${seed.title}»")
            }
        }
        // a mood: more tracks of it by a search (the words of the mood, in the language of the likes)
        if (vibe != "any") VIBE_WORDS[vibe]?.let { w ->
            val ru = liked.count { l -> l.title.any { it in 'Ѐ'..'ӿ' } } * 2 >= liked.size
            val q = if (ru) w.queryRu else w.queryEn
            try {
                app.api.search(q, offset = rng.nextInt(0, 3) * 20).forEach { t ->
                    hits[t.id] = t to ((hits[t.id]?.second ?: 0) + 1)
                    via.putIfAbsent(t.id, "по настроению")
                }
            } catch (e: Exception) { /* the seeds are enough */ }
        }

        // with the liked allowed, some of them join the pool
        if (!noLiked) liked.shuffled().take(30).forEach { t -> hits[t.id] = t to ((hits[t.id]?.second ?: 0) + 1); via.putIfAbsent(t.id, "из ваших лайков") }
        val heard = if (noHeard) History.all(app.store).map { it.id }.toHashSet() else emptySet()
        val knownArtists = liked.map { it.artistId }.filter { it != 0L }.toHashSet()
        val dislikedIds = dislikedList.map { it.id }.toHashSet()
        fun cyr(t: Track) = (t.title + t.artist).any { it in 'Ѐ'..'ӿ' }
        val pool = hits.values.filter { (t, _) ->
            (!noLiked || t.id !in likedIds) && t.id !in played && t.id !in heard && t.id !in dislikedIds && t.artistId !in bannedArtists.keys && t.durationMs in 30_000..15 * 60_000 &&
                (language == 0 || (language == 1) == cyr(t)) &&
                (novelty == 0 || (novelty == 1) == (t.artistId !in knownArtists))
        }
        val weighted = weightedPick(pool, n * 3) { (t, count) ->
            val recent = recentArtists.indexOf(t.artistId)
            val artistFactor = when { t.artistId == 0L || recent < 0 -> 1.0; recent < 12 -> 0.12; else -> 0.45 }
            (1.0 + 0.6 * (count - 1)) * Vibes.factor(vibe, t.words) * artistFactor
        }.map { it.first }
        // no more than two songs of one artist, and not twice in a row
        val per = HashMap<Long, Int>()
        val chosen = ArrayList<Track>()
        for (t in weighted) {
            if (chosen.size >= n) break
            val c = per.getOrDefault(t.artistId, 0)
            if (t.artistId == 0L || c < 2) {
                per[t.artistId] = c + 1
                chosen += t
            }
        }
        spread(chosen)
        for (t in chosen) {
            played += t.id
            recentArtists.addFirst(t.artistId)
            reasons[t.id] = via[t.id] ?: "из ваших лайков"
        }
        while (recentArtists.size > 24) recentArtists.removeLast()
        while (played.size > 3000) played.remove(played.first())
        return chosen
    }

    /** Efraimidis-Spirakis: a weighted sample without repeats (the keys u^(1/w), the biggest first). */
    private fun <T> weightedPick(items: List<T>, k: Int, weight: (T) -> Double): List<T> =
        items.map { it to rng.nextDouble(1e-12, 1.0).pow(1.0 / weight(it).coerceAtLeast(1e-6)) }
            .sortedByDescending { it.second }.take(k).map { it.first }

    private fun spread(list: MutableList<Track>) {
        for (i in 1 until list.size) {
            if (list[i].artistId != 0L && list[i].artistId == list[i - 1].artistId) {
                val j = (i + 1 until list.size).firstOrNull { list[it].artistId != list[i - 1].artistId } ?: continue
                val t = list[i]; list[i] = list[j]; list[j] = t
            }
        }
    }
}
