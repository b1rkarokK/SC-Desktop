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
