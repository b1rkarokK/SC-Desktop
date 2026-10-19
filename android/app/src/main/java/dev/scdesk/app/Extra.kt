package dev.scdesk.app

import android.os.SystemClock
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.Request
import org.json.JSONArray
import org.json.JSONObject
import kotlinx.coroutines.launch

// ------------------------------------------------------------------ history

/** A play that happened on this phone: kept on the phone only. */
data class Played(val id: Long, val title: String, val artist: String, val artwork: String?, val durationMs: Long, val at: Long)

/** The plays of this phone (the last 600): the history and the totals are made from them. */
object History {
    fun all(store: Store): List<Played> = try {
        val a = JSONArray(store.history)
        (0 until a.length()).map { a.getJSONObject(it) }.map { Played(it.getLong("i"), it.optString("t"), it.optString("a"), it.optString("c").ifEmpty { null }, it.optLong("d"), it.optLong("at")) }
    } catch (e: Exception) {
        emptyList()
    }

    @Synchronized
    fun add(store: Store, t: Track) {
        val now = System.currentTimeMillis()
        val list = all(store)
        // the same track started again at once (a seek back to the start, a redraw) is not a new play
        if (list.lastOrNull()?.let { it.id == t.id && now - it.at < 30_000 } == true) return
        val a = JSONArray()
        (list + Played(t.id, t.title, t.artist, t.artwork, t.durationMs, now)).takeLast(600).forEach {
            a.put(JSONObject().put("i", it.id).put("t", it.title).put("a", it.artist).put("c", it.artwork ?: "").put("d", it.durationMs).put("at", it.at))
        }
        store.history = a.toString()
    }
}

@Composable
private fun PageHead(title: String, onBack: () -> Unit) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(44.dp).pressable(onBack), contentAlignment = Alignment.Center) { Icon(Ico.Back, 26.dp) }
        Text(title, color = C.text, fontSize = 20.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
internal fun Page(onBack: () -> Unit, title: String, content: @Composable () -> Unit) {
    Column(Modifier.fillMaxSize().background(C.bg).statusBarsPadding().navigationBarsPadding().pressable { }) {
        PageHead(title, onBack)
        content()
    }
}

/** «СЕГОДНЯ · ВЕЧЕР»: the day and the part of the day of a play. */
private fun bucket(at: Long): String {
    val now = java.util.Calendar.getInstance()
    val c = java.util.Calendar.getInstance().apply { timeInMillis = at }
    val days = ((now.apply { set(java.util.Calendar.HOUR_OF_DAY, 0); set(java.util.Calendar.MINUTE, 0); set(java.util.Calendar.SECOND, 0) }.timeInMillis - java.util.Calendar.getInstance().apply { timeInMillis = at; set(java.util.Calendar.HOUR_OF_DAY, 0); set(java.util.Calendar.MINUTE, 0); set(java.util.Calendar.SECOND, 0) }.timeInMillis) / 86_400_000L).toInt()
    val day = when (days) { 0 -> "СЕГОДНЯ"; 1 -> "ВЧЕРА"; else -> java.text.SimpleDateFormat("d MMMM", java.util.Locale("ru")).format(java.util.Date(at)).uppercase() }
    val h = c.get(java.util.Calendar.HOUR_OF_DAY)
    return day + " · " + when { h < 6 -> "НОЧЬ"; h < 12 -> "УТРО"; h < 18 -> "ДЕНЬ"; else -> "ВЕЧЕР" }
}

@Composable
internal fun HistoryScreen(app: App, shown: Shown, play: (List<Track>, Int) -> Unit, onBack: () -> Unit) {
    val all = remember { History.all(app.store).asReversed() }
    var filter by remember { mutableStateOf("") }
    val items = if (filter.isBlank()) all else all.filter { it.title.contains(filter, true) || it.artist.contains(filter, true) }
    Page(onBack, "История") {
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp).clip(RoundedCornerShape(14.dp)).background(C.panel).padding(horizontal = 14.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            SIcon("search", C.muted)
            Box(Modifier.padding(start = 10.dp).weight(1f)) {
                if (filter.isEmpty()) Text("Название или исполнитель", color = C.faint, fontSize = 15.sp)
                androidx.compose.foundation.text.BasicTextField(filter, { filter = it }, singleLine = true, textStyle = androidx.compose.ui.text.TextStyle(color = C.text, fontSize = 15.sp), cursorBrush = androidx.compose.ui.graphics.SolidColor(C.accent), modifier = Modifier.fillMaxWidth())
            }
        }
        if (items.isEmpty()) {
            Text(if (all.isEmpty()) "Здесь будет то, что вы слушали на телефоне." else "Ничего не нашлось.", color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(20.dp))
        } else {
            val tracks = items.map { Track(it.id, it.title, it.artist, it.durationMs, it.artwork) }
            var last = ""
            LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 96.dp)) {
                itemsIndexed(tracks, key = { i, t -> "$i-${t.id}" }) { i, t ->
                    val b = bucket(items[i].at)
                    val head = if (i == 0 || bucket(items[i - 1].at) != b) b else null
                    Column {
                        head?.let { Text(it, color = C.muted, fontSize = 11.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(start = 20.dp, top = 14.dp, bottom = 2.dp)) }
                        TrackRow(t.copy(artist = t.artist + " · " + java.text.SimpleDateFormat("HH:mm", java.util.Locale.getDefault()).format(java.util.Date(items[i].at))), shown.track?.id == t.id) { play(tracks, i) }
                    }
                }
            }
        }
    }
}

// -------------------------------------------------------------------- stats

private fun plural(n: Int) = if (n % 10 == 1 && n % 100 != 11) "раз" else if (n % 10 in 2..4 && n % 100 !in 12..14) "раза" else "раз"

private fun spent(ms: Long): String {
    val m = ms / 60_000
    return if (m >= 60) "${m / 60} ч ${m % 60} мин" else "$m мин"
}

@Composable
internal fun StatsScreen(app: App, onBack: () -> Unit) {
    // 0 week, 1 month, 2 year
    var range by remember { mutableIntStateOf(0) }
    val all = remember { History.all(app.store) }
    val day = 86_400_000L
    val span = when (range) { 0 -> 7 * day; 1 -> 30 * day; else -> 365 * day }
    val now = System.currentTimeMillis()
    val list = all.filter { it.at >= now - span }
    val before = all.filter { it.at < now - span && it.at >= now - 2 * span }
    Page(onBack, "Итоги") {
        Row(Modifier.padding(horizontal = 16.dp, vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf("Неделя", "Месяц", "Год").forEachIndexed { i, l ->
                Text(
                    l, color = if (range == i) C.bg else C.text, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                    modifier = Modifier.weight(1f).clip(RoundedCornerShape(14.dp)).background(if (range == i) C.accent else C.panel).pressable { range = i }.padding(vertical = 12.dp),
                )
            }
        }
        if (list.isEmpty()) {
            Text("Пока нечего считать: слушайте музыку на телефоне, и здесь появятся итоги.", color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(20.dp))
            return@Page
        }
        val total = list.sumOf { it.durationMs }
        val diff = total - before.sumOf { it.durationMs }
        // the bars: the days of the week, the weeks of the month, the months of the year
        val labels: List<String>
        val sums: List<Long>
        when (range) {
            0 -> {
                labels = listOf("Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс")
                val s = LongArray(7)
                list.forEach { p -> val d = java.util.Calendar.getInstance().apply { timeInMillis = p.at }.get(java.util.Calendar.DAY_OF_WEEK); s[(d + 5) % 7] += p.durationMs }
                sums = s.toList()
            }
            1 -> {
                labels = listOf("4 нед. назад", "3", "2", "прошлая", "эта")
                val s = LongArray(5)
                list.forEach { p -> val w = ((now - p.at) / (7 * day)).toInt().coerceIn(0, 4); s[4 - w] += p.durationMs }
                sums = s.toList()
            }
            else -> {
                labels = listOf("Я", "Ф", "М", "А", "М", "И", "И", "А", "С", "О", "Н", "Д")
                val s = LongArray(12)
                list.forEach { p -> s[java.util.Calendar.getInstance().apply { timeInMillis = p.at }.get(java.util.Calendar.MONTH)] += p.durationMs }
                sums = s.toList()
            }
        }
        val top = sums.maxOrNull()?.coerceAtLeast(1) ?: 1
        Column(Modifier.verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
            Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp).clip(RoundedCornerShape(18.dp)).background(C.panel).padding(16.dp)) {
                Text(spent(total), color = C.text, fontSize = 26.sp, fontWeight = FontWeight.Bold)
                Text(
                    "вы слушали за " + listOf("неделю", "месяц", "год")[range] + if (before.isEmpty()) "" else if (diff >= 0) ", на ${spent(diff)} больше прошлой" else ", на ${spent(-diff)} меньше прошлой",
                    color = C.muted, fontSize = 13.sp, modifier = Modifier.padding(top = 2.dp),
                )
                Spacer(Modifier.height(18.dp))
                Row(Modifier.fillMaxWidth().height(96.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.Bottom) {
                    sums.forEachIndexed { i, v ->
                        Box(Modifier.weight(1f).height((14 + 82f * v / top).dp).clip(RoundedCornerShape(8.dp)).background(if (v == top) C.accent else C.panel2))
                    }
                }
                Row(Modifier.fillMaxWidth().padding(top = 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    labels.forEach { Text(it, color = C.faint, fontSize = 10.sp, textAlign = androidx.compose.ui.text.style.TextAlign.Center, maxLines = 1, modifier = Modifier.weight(1f)) }
                }
            }
            Text("ЛЮБИМЫЕ АРТИСТЫ", color = C.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(start = 20.dp, top = 18.dp, bottom = 6.dp))
            list.groupBy { it.artist }.entries.sortedByDescending { e -> e.value.sumOf { it.durationMs } }.take(5).forEach { (name, plays) ->
                Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 7.dp), verticalAlignment = Alignment.CenterVertically) {
                    Box(Modifier.size(44.dp).clip(androidx.compose.foundation.shape.CircleShape).background(C.panel2), contentAlignment = Alignment.Center) { Text(name.take(1).uppercase(), color = C.text, fontSize = 16.sp, fontWeight = FontWeight.SemiBold) }
                    Column(Modifier.padding(start = 12.dp).weight(1f)) {
                        Text(name.ifEmpty { "Без имени" }, color = C.text, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(spent(plays.sumOf { it.durationMs }) + " · " + plays.size + " " + plural(plays.size), color = C.muted, fontSize = 12.sp)
                    }
                }
            }
            val again = list.groupBy { it.id }.values.filter { it.size > 1 }.sortedByDescending { it.size }.take(3)
            if (again.isNotEmpty()) {
                Text("ПЕРЕСЛУШИВАЕТЕ", color = C.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(start = 20.dp, top = 18.dp, bottom = 6.dp))
                again.forEach { g ->
                    Column(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 6.dp)) {
                        Text(g[0].title, color = C.text, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(g[0].artist + " · " + g.size + " " + plural(g.size), color = C.muted, fontSize = 12.sp)
                    }
                }
            }
        }
    }
}

// ----------------------------------------------------------------- settings helpers (the screens are in Settings.kt)

@Composable
internal fun Item(title: String, sub: String, onClick: () -> Unit) {
    Column(Modifier.fillMaxWidth().pressable(onClick).padding(horizontal = 20.dp, vertical = 14.dp)) {
        Text(title, color = C.text, fontSize = 16.sp, fontWeight = FontWeight.Medium)
        Text(sub, color = C.muted, fontSize = 13.sp)
    }
}

@Composable
internal fun Toggle(title: String, sub: String, on: Boolean, onChange: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth().pressable { onChange(!on) }.padding(horizontal = 20.dp, vertical = 14.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(title, color = C.text, fontSize = 16.sp, fontWeight = FontWeight.Medium)
            Text(sub, color = C.muted, fontSize = 13.sp)
        }
        Box(Modifier.size(width = 48.dp, height = 28.dp).clip(RoundedCornerShape(14.dp)).background(if (on) C.accent else C.panel2).padding(3.dp)) {
            Box(Modifier.align(if (on) Alignment.CenterEnd else Alignment.CenterStart).size(22.dp).clip(RoundedCornerShape(11.dp)).background(C.text))
        }
    }
}

// ------------------------------------------------------------------- lyrics

data class LyricWord(val start: Long, val end: Long, val text: String)
data class LyricLine(val atMs: Long, val text: String, val words: List<LyricWord> = emptyList())
data class Lyrics(val lines: List<LyricLine>, val synced: Boolean)

/** The lyrics from lrclib.net (the free base the computer uses too); the lines have times when the base has them. */
object LyricsApi {
    private val stamp = Regex("""\[(\d+):(\d+)(?:[.:](\d+))?]""")

    fun parse(synced: String?, plain: String?): Lyrics? {
        if (!synced.isNullOrBlank()) {
            val out = ArrayList<LyricLine>()
            for (raw in synced.lines()) {
                val m = stamp.findAll(raw).toList()
                if (m.isEmpty()) continue
                val text = raw.substring(m.last().range.last + 1).trim()
                for (s in m) {
                    val frac = s.groupValues[3]
                    val ms = s.groupValues[1].toLong() * 60_000 + s.groupValues[2].toLong() * 1000 + (if (frac.isEmpty()) 0 else (frac + "00").take(3).toLong())
                    out.add(LyricLine(ms, text))
                }
            }
            if (out.isNotEmpty()) return Lyrics(out.sortedBy { it.atMs }, true)
        }
        if (!plain.isNullOrBlank()) return Lyrics(plain.lines().map { LyricLine(0, it.trim()) }, false)
        return null
    }

    /** The lyrics of the computer (it may have the words with their times); null when it is away or has nothing. */
    suspend fun fromComputer(app: App, t: Track, force: Boolean = false): Lyrics? {
        if (!app.store.linked) return null
        // a search again takes the computer longer: it asks every source once more
        val d = runCatching { app.link.rpc("lyrics", JSONObject().put("track", t.id).put("force", force), waitMs = if (force) 30_000 else 1_500).getJSONObject("data") }.getOrNull() ?: return null
        if (!d.optBoolean("found")) return null
        val arr = d.optJSONArray("lines") ?: return null
        val lines = (0 until arr.length()).map { arr.getJSONObject(it) }.map { l ->
            val ws = l.optJSONArray("w")
            LyricLine(
                if (l.isNull("s")) 0L else l.optLong("s"), l.optString("t"),
                if (ws == null) emptyList() else (0 until ws.length()).map { ws.getJSONObject(it) }.map { LyricWord(it.optLong("s"), it.optLong("e"), it.optString("t")) },
            )
        }
        return Lyrics(lines, d.optBoolean("synced"))
    }

    /** The line with its words coloured by where the song is: sung ones, the one that sounds, the coming ones. */
    fun coloured(line: LyricLine, pos: Long, sung: androidx.compose.ui.graphics.Color, now: androidx.compose.ui.graphics.Color, coming: androidx.compose.ui.graphics.Color): androidx.compose.ui.text.AnnotatedString =
        androidx.compose.ui.text.buildAnnotatedString {
            if (line.words.isEmpty()) { append(line.text); return@buildAnnotatedString }
            line.words.forEachIndexed { i, w ->
                val c = if (pos >= w.end) sung else if (pos >= w.start) now else coming
                pushStyle(androidx.compose.ui.text.SpanStyle(color = c))
                append(w.text.trim())
                pop()
                if (i < line.words.size - 1) append(" ")
            }
        }

    /** Blocking. Null when the base has nothing. */
    fun fetch(t: Track): Lyrics? {
        val title = t.title.replace(Regex("""\s*[\(\[].*?(feat|ft|prod|remix|official|lyric|video).*?[\)\]]""", RegexOption.IGNORE_CASE), "").trim()
        val url = "https://lrclib.net/api/get".toHttpUrl().newBuilder()
            .addQueryParameter("track_name", title).addQueryParameter("artist_name", t.artist)
            .apply { if (t.durationMs > 0) addQueryParameter("duration", (t.durationMs / 1000).toString()) }.build()
        val got = ask(url.toString())?.let { JSONObject(it) }
        parse(got?.optString("syncedLyrics"), got?.optString("plainLyrics"))?.let { return it }
        val search = "https://lrclib.net/api/search".toHttpUrl().newBuilder().addQueryParameter("q", "${t.artist} $title").build()
        val arr = ask(search.toString())?.let { JSONArray(it) } ?: return null
        for (i in 0 until minOf(arr.length(), 5)) {
            val o = arr.getJSONObject(i)
            parse(o.optString("syncedLyrics"), o.optString("plainLyrics"))?.let { return it }
        }
        return null
    }

    private fun ask(url: String): String? =
        http.newCall(Request.Builder().url(url).header("User-Agent", USER_AGENT).build()).execute().use { r -> if (r.isSuccessful) r.body?.string() else null }
}

@Composable
internal fun LyricsScreen(app: App, shown: Shown, t: Track, onBack: () -> Unit) {
    var result by remember(t.id) { mutableStateOf<Lyrics?>(null) }
    var state by remember(t.id) { mutableStateOf("Ищем текст…") }
    LaunchedEffect(t.id) {
        try {
            result = LyricsApi.fromComputer(app, t) ?: withContext(Dispatchers.IO) { LyricsApi.fetch(t) }
            if (result == null) state = "Текста для этого трека не нашлось."
        } catch (e: Exception) {
            state = "Не удалось загрузить текст: ${e.message}"
        }
    }
    var sizeStep by remember { mutableIntStateOf(0) }
    val lctx = androidx.compose.ui.platform.LocalContext.current
    // a translation of the lines under them (the computer does it, as on its own screen)
    var translated by remember(t.id) { mutableStateOf<List<String>?>(null) }
    var translating by remember(t.id) { mutableStateOf(false) }
    val tscope = androidx.compose.runtime.rememberCoroutineScope()
    val remote = shown.remote
    val pos by produceState(0L, remote?.dev, shown.playing, t.id) {
        while (true) {
            value = if (remote != null) remote.posMs + if (remote.playing) SystemClock.elapsedRealtime() - remote.posAt else 0 else app.engine.position()
            delay(300)
        }
    }
    Page(onBack, t.title) {
        val ly = result
        if (ly == null) {
            Text(state, color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(20.dp))
            return@Page
        }
        Row(Modifier.padding(horizontal = 24.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(18.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("A−", color = C.accent, fontSize = 16.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.pressable { sizeStep = (sizeStep - 1).coerceAtLeast(-2) }.padding(6.dp))
            Text("A+", color = C.accent, fontSize = 16.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.pressable { sizeStep = (sizeStep + 1).coerceAtMost(3) }.padding(6.dp))
            Text("Скопировать текст", color = C.accent, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.pressable {
                (lctx.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager)
                    .setPrimaryClip(android.content.ClipData.newPlainText("lyrics", ly.lines.joinToString("\n") { it.text }))
                android.widget.Toast.makeText(lctx, L.t("Текст скопирован"), android.widget.Toast.LENGTH_SHORT).show()
            }.padding(6.dp))
        }
        if (app.store.linked) {
            Text(
                if (translating) "Переводим…" else if (translated != null) "Убрать перевод" else "Перевести на русский",
                color = C.accent, fontSize = 14.sp, fontWeight = FontWeight.SemiBold,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp).pressable {
                    if (translating) return@pressable
                    if (translated != null) { translated = null; return@pressable }
                    translating = true
                    tscope.launch {
                        runCatching {
                            val lines = org.json.JSONArray().also { a -> ly.lines.forEach { a.put(it.text) } }
                            val d = app.link.rpc("translate", JSONObject().put("track", t.id).put("lines", lines).put("to", "ru"), waitMs = 60_000).getJSONArray("data")
                            translated = (0 until d.length()).map { d.optString(it) }
                        }
                        translating = false
                    }
                },
            )
        }
        val current = if (ly.synced) ly.lines.indexOfLast { it.atMs <= pos + 250 } else -1
        val list = rememberLazyListState()
        LaunchedEffect(current) { if (current >= 0) list.animateScrollToItem(maxOf(current - 2, 0)) }
        LazyColumn(Modifier.fillMaxSize(), state = list, contentPadding = PaddingValues(horizontal = 24.dp, vertical = 12.dp)) {
            itemsIndexed(ly.lines) { i, l ->
                val on = !ly.synced || i == current
                Text(
                    if (i == current && l.words.isNotEmpty()) LyricsApi.coloured(l, pos, C.text, C.accent, C.faint) else androidx.compose.ui.text.AnnotatedString(l.text.ifEmpty { " " }), color = if (on) C.text else C.faint, fontSize = (if (i == current) 24 else 20).plus(sizeStep * 2).sp, fontWeight = if (i == current) FontWeight.Bold else FontWeight.Medium,
                    modifier = Modifier.fillMaxWidth().padding(vertical = 6.dp).let { m -> if (ly.synced && remote == null) m.pressable { app.engine.seek(l.atMs) } else m },
                )
                translated?.getOrNull(i)?.takeIf { it.isNotBlank() && it != l.text }?.let { Text(it, color = C.muted, fontSize = 14.sp, modifier = Modifier.fillMaxWidth().padding(bottom = 4.dp)) }
            }
            item { Spacer(Modifier.height(96.dp)) }
        }
    }
}

// ---------------------------------------------------------------- downloads

/** «Скачанное»: how much room is taken, and the tracks kept for the offline. */
@Composable
internal fun DownloadsScreen(app: App, shown: Shown, play: (List<Track>, Int) -> Unit, onBack: () -> Unit) {
    var tick by remember { mutableIntStateOf(0) }
    val items = remember(tick) { Downloads.all() }
    val used = remember(tick) { Downloads.usedBytes() }
    val limit = app.store.downloadLimitGb * 1_000_000_000L
    Page(onBack, "Скачанное") {
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp).clip(RoundedCornerShape(16.dp)).background(C.panel).padding(16.dp)) {
            Text("${fmtSize(used)} из ${app.store.downloadLimitGb} ГБ", color = C.text, fontSize = 17.sp, fontWeight = FontWeight.SemiBold)
            Box(Modifier.padding(vertical = 10.dp).fillMaxWidth().height(6.dp).clip(RoundedCornerShape(3.dp)).background(C.panel2)) {
                Box(Modifier.fillMaxWidth((used.toFloat() / limit).coerceIn(0f, 1f)).height(6.dp).background(C.accent))
            }
            Text("Лимит меняется в настройках", color = C.muted, fontSize = 12.sp)
        }
        if (items.isEmpty()) {
            Text("Скачанных треков пока нет. В плеере, в меню «⋮»: «Скачать для офлайна».", color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(20.dp))
        } else {
            Text("ДЛЯ ОФЛАЙНА", color = C.muted, fontSize = 11.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(start = 20.dp, top = 14.dp, bottom = 2.dp))
            val tracks = items.map { it.track }
            LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 96.dp)) {
                itemsIndexed(tracks, key = { _, t -> t.id }) { i, t ->
                    TrackRow(t.copy(artist = t.artist + " · Загружено · " + fmtSize(items[i].size)), shown.track?.id == t.id, trailing = {
                        // the mark takes the track out of the saved ones
                        Box(Modifier.size(40.dp).pressable { Downloads.remove(app, t.id); tick++ }, contentAlignment = Alignment.Center) { Icon(Ico.Check, 24.dp, C.accent) }
                    }) { play(tracks, i) }
                }
            }
        }
    }
}
