package dev.scdesk.app

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

/** A playlist of the user: its number, name and how many tracks it holds. */
private data class MyList(val id: Long, val title: String, val n: Int, val channel: String = "")

/**
 * «Добавить в плейлист»: the user's own playlists (the computer lists them and does the writing, it holds the session
 * that writes to SoundCloud), a new playlist with this track in it, or a click on a playlist to put the track into it.
 */
@Composable
internal fun PlaylistPicker(app: App, track: Track, shared: Boolean = false, onClose: () -> Unit) {
    val scope = rememberCoroutineScope()
    val ctx = androidx.compose.ui.platform.LocalContext.current
    var lists by remember { mutableStateOf<List<MyList>?>(null) }
    var note by remember { mutableStateOf<String?>(null) }
    var creating by remember { mutableStateOf(false) }
    var title by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    fun toast(s: String) = android.widget.Toast.makeText(ctx, s, android.widget.Toast.LENGTH_SHORT).show()

    LaunchedEffect(Unit) {
        if (!shared) {
            // the user's own playlists: from SoundCloud itself
            val own = runCatching { withContext(kotlinx.coroutines.Dispatchers.IO) { app.api.ownPlaylists() } }.getOrNull()
            if (own != null) { lists = own.map { MyList(it.first, it.second, it.third) }; return@LaunchedEffect }
        }
        if (!app.store.linked) { note = "Нужен подключённый компьютер для общих плейлистов."; lists = emptyList(); return@LaunchedEffect }
        runCatching { app.link.rpc(if (shared) "sp_list" else "my_playlists").getJSONArray("data") }
            .onSuccess { a ->
                val objs = (0 until a.length()).map { a.getJSONObject(it) }
                // shared playlists: only those the user may add to
                lists = if (shared) objs.filter { it.optBoolean("can_edit") }.map { MyList(0, it.optString("title"), it.optInt("count"), it.optString("channel")) }
                else objs.map { MyList(it.getLong("id"), it.optString("title"), it.optInt("n")) }
            }
            .onFailure { lists = emptyList(); note = "Не получилось: ${it.message}" }
    }

    Box(Modifier.fillMaxSize().background(Color(0xAA000000)).pressable(onClose)) {
        Column(
            Modifier.align(Alignment.BottomCenter).fillMaxWidth().heightIn(max = 520.dp).clip(RoundedCornerShape(topStart = 24.dp, topEnd = 24.dp)).background(C.panel)
                .consumeTaps().navigationBarsPadding().padding(top = 16.dp, bottom = 12.dp),
        ) {
            Text(if (shared) "В общий плейлист" else "В плейлист", color = C.text, fontSize = 18.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(start = 20.dp))
            Text(track.title, color = C.muted, fontSize = 13.sp, maxLines = 1, modifier = Modifier.padding(start = 20.dp, end = 20.dp, bottom = 8.dp))
            note?.let { Text(it, color = C.accent, fontSize = 13.sp, modifier = Modifier.padding(horizontal = 20.dp, vertical = 4.dp)) }
            Column(Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState())) {
                if (creating) {
                    Box(Modifier.padding(horizontal = 20.dp, vertical = 6.dp).fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(C.panel2).padding(horizontal = 14.dp, vertical = 12.dp)) {
                        if (title.isEmpty()) Text("Название плейлиста", color = C.faint, fontSize = 15.sp)
                        BasicTextField(title, { title = it }, singleLine = true, textStyle = TextStyle(color = C.text, fontSize = 15.sp), cursorBrush = SolidColor(C.accent), modifier = Modifier.fillMaxWidth())
                    }
                    Row(Modifier.padding(horizontal = 20.dp)) {
                        Text(if (busy) "Создаём…" else "Создать", color = C.accent, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.pressable {
                            if (busy || title.isBlank()) return@pressable
                            busy = true
                            scope.launch {
                                runCatching { Writes.call(app, "pl_create", JSONObject().put("title", title.trim()).put("track", track.id)) }
                                    .onSuccess { toast("Плейлист «${title.trim()}» создан"); onClose() }
                                    .onFailure { note = "Не вышло: ${it.message}"; busy = false }
                            }
                        }.padding(vertical = 10.dp))
                    }
                } else if (!shared) {
                    Text("＋ Новый плейлист", color = C.accent, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.fillMaxWidth().pressable { creating = true }.padding(horizontal = 20.dp, vertical = 12.dp))
                }
                when (val l = lists) {
                    null -> Text("Загружаем…", color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(20.dp))
                    else -> l.forEach { p ->
                        Row(Modifier.fillMaxWidth().pressable {
                            if (busy) return@pressable
                            busy = true
                            scope.launch {
                                runCatching { if (shared) app.link.rpc("sp_add", JSONObject().put("channel", p.channel).put("track", track.id)) else Writes.call(app, "pl_add", JSONObject().put("playlist", p.id).put("track", track.id)) }
                                    .onSuccess { toast("Добавлено в «${p.title}»"); onClose() }
                                    .onFailure { note = it.message ?: "Не вышло"; busy = false }
                            }
                        }.padding(horizontal = 20.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                            Text(p.title, color = C.text, fontSize = 16.sp, maxLines = 1, modifier = Modifier.weight(1f))
                            Text("${p.n}", color = C.muted, fontSize = 13.sp)
                        }
                    }
                }
            }
            Spacer(Modifier.height(4.dp))
        }
    }
}

/**
 * «Удалить плейлист»: asked first (a long press on the playlist). The computer deletes it; a playlist that is not the
 * user's own (a liked one) cannot be deleted, the computer says so.
 */
@Composable
internal fun DeletePlaylist(app: App, id: Long, title: String, onClose: () -> Unit, onDeleted: () -> Unit) {
    val scope = rememberCoroutineScope()
    val ctx = androidx.compose.ui.platform.LocalContext.current
    var busy by remember { mutableStateOf(false) }
    androidx.compose.ui.window.Dialog(onDismissRequest = onClose) {
        Column(Modifier.clip(RoundedCornerShape(20.dp)).background(C.panel).padding(20.dp)) {
            Text("Удалить плейлист?", color = C.text, fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
            Text("«$title» будет удалён из вашего аккаунта SoundCloud.", color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(top = 8.dp, bottom = 14.dp))
            Row {
                Text("Отмена", color = C.muted, fontSize = 15.sp, modifier = Modifier.pressable(onClose).padding(12.dp))
                Spacer(Modifier.weight(1f))
                Text(if (busy) "Удаляем…" else "Удалить", color = C.accent, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.pressable {
                    if (busy) return@pressable
                    busy = true
                    scope.launch {
                        runCatching { Writes.call(app, "pl_delete", JSONObject().put("playlist", id)) }
                            .onSuccess { onDeleted(); onClose() }
                            .onFailure { android.widget.Toast.makeText(ctx, L.t("Не получилось: {0}").replace("{0}", it.message ?: ""), android.widget.Toast.LENGTH_LONG).show(); busy = false }
                    }
                }.padding(12.dp))
            }
        }
    }
}

/**
 * What can be done with a song of a list (a long press on it): the queue, a playlist, a link, the download, the likes.
 * When the music plays on the computer, the queue is the computer's.
 */
@Composable
internal fun TrackActions(app: App, t: Track, onComputer: Boolean, fromPlaylist: Long? = null, onRemoved: () -> Unit = {}, onRemovedLike: () -> Unit = {}, onMove: ((Int) -> Unit)? = null, onClose: () -> Unit) {
    val scope = rememberCoroutineScope()
    val ctx = androidx.compose.ui.platform.LocalContext.current
    var picker by remember { mutableStateOf(0) } // 1 own playlists, 2 shared ones, 3 similar songs
    fun toast(s: String) = android.widget.Toast.makeText(ctx, L.t(s), android.widget.Toast.LENGTH_SHORT).show()
    if (picker == 3) {
        SimilarDialog(app, t, onClose)
        return
    }
    if (picker == 4) {
        TrackPageDialog(app, t, onClose)
        return
    }
    if (picker != 0) {
        androidx.compose.ui.window.Dialog(onDismissRequest = onClose, properties = androidx.compose.ui.window.DialogProperties(usePlatformDefaultWidth = false)) {
            PlaylistPicker(app, t, shared = picker == 2) { onClose() }
        }
        return
    }
    val have = Downloads.localFile(t.id) != null
    val items = listOf<Pair<String, () -> Unit>>(
        "Играть следующим" to {
            if (onComputer) scope.launch { runCatching { app.link.rpc("enqueue", JSONObject().put("track", t.id).put("next", true)) } } else app.engine.insertNext(t)
            toast("Сыграет следующим"); onClose()
        },
        "В конец очереди" to {
            if (onComputer) scope.launch { runCatching { app.link.rpc("enqueue", JSONObject().put("track", t.id).put("next", false)) } } else app.engine.append(listOf(t))
            toast("Добавлено в конец очереди"); onClose()
        },
        "Страница трека" to { picker = 4 },
        "Похожие треки" to { picker = 3 },
        "Добавить в плейлист…" to { picker = 1 },
        "Выше в плейлисте" to { onMove?.invoke(-1); onClose() },
        "Ниже в плейлисте" to { onMove?.invoke(1); onClose() },
        "В общий плейлист…" to { picker = 2 },
        "Поделиться ссылкой" to {
            scope.launch {
                val url = withContext(kotlinx.coroutines.Dispatchers.IO) { app.api.permalink(t.id) }
                if (url == null) toast("Не получилось")
                else ctx.startActivity(android.content.Intent.createChooser(android.content.Intent(android.content.Intent.ACTION_SEND).setType("text/plain").putExtra(android.content.Intent.EXTRA_TEXT, url), null).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK))
            }
            onClose()
        },
        "Открыть на SoundCloud" to {
            scope.launch {
                val url = withContext(kotlinx.coroutines.Dispatchers.IO) { app.api.permalink(t.id) }
                if (url == null) toast("Не получилось")
                else ctx.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(url)).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK))
            }
            onClose()
        },
        (if (have) "Удалить из скачанного" else "Скачать для офлайна") to {
            if (have) Downloads.remove(app, t.id)
            else scope.launch {
                val msg = withContext(kotlinx.coroutines.Dispatchers.IO) { try { Downloads.save(ctx, app, t); "Скачано: ${t.title}" } catch (e: Exception) { e.message ?: "Не вышло скачать" } }
                android.widget.Toast.makeText(ctx, msg, android.widget.Toast.LENGTH_LONG).show()
            }
            onClose()
        },
        "Убрать из этого плейлиста" to {
            scope.launch {
                runCatching { Writes.call(app, "pl_remove", JSONObject().put("playlist", fromPlaylist ?: 0L).put("track", t.id)) }
                    .onSuccess { onRemoved(); toast("Убрано из плейлиста") }
                    .onFailure { android.widget.Toast.makeText(ctx, L.t("Не получилось: {0}").replace("{0}", it.message ?: ""), android.widget.Toast.LENGTH_LONG).show() }
            }
            onClose()
        },
        "Убрать из лайков" to {
            scope.launch { runCatching { Writes.call(app, "like", JSONObject().put("track", t.id).put("on", false)) } }
            onRemovedLike(); toast("Убрано из лайков"); onClose()
        },
    )
    androidx.compose.ui.window.Dialog(onDismissRequest = onClose) {
        Column(Modifier.clip(RoundedCornerShape(20.dp)).background(C.panel).padding(vertical = 14.dp)) {
            Text(t.title, color = C.text, fontSize = 16.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, modifier = Modifier.padding(horizontal = 20.dp))
            Text(t.artist, color = C.muted, fontSize = 13.sp, maxLines = 1, modifier = Modifier.padding(start = 20.dp, end = 20.dp, bottom = 8.dp))
            Column(Modifier.heightIn(max = 420.dp).verticalScroll(rememberScrollState())) {
                items.filter { (it.first != "Убрать из этого плейлиста" || fromPlaylist != null) && (it.first !in listOf("Выше в плейлисте", "Ниже в плейлисте") || onMove != null) }.forEach { (label, act) -> Text(label, color = C.text, fontSize = 15.sp, modifier = Modifier.fillMaxWidth().pressable(act).padding(horizontal = 20.dp, vertical = 13.dp)) }
            }
        }
    }
}

/**
 * «Дубли» in the likes: the same song liked more than once (another upload of it, or the same one twice). The groups are
 * shown, «Убрать лишние» takes the likes back from all but the earliest of each. Done by the phone itself.
 */
@Composable
internal fun DuplicatesDialog(app: App, onClose: () -> Unit, onRemoved: (Set<Long>) -> Unit) {
    val scope = rememberCoroutineScope()
    val ctx = androidx.compose.ui.platform.LocalContext.current
    var groups by remember { mutableStateOf<List<List<Track>>?>(null) }
    var busy by remember { mutableStateOf(false) }
    var note by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(Unit) {
        groups = runCatching { withContext(kotlinx.coroutines.Dispatchers.IO) { Independent.duplicates(app.api.likesDated(5000)) } }
            .onFailure { note = "Не получилось: ${it.message}" }.getOrDefault(emptyList())
    }
    androidx.compose.ui.window.Dialog(onDismissRequest = onClose) {
        Column(Modifier.clip(RoundedCornerShape(20.dp)).background(C.panel).padding(vertical = 16.dp)) {
            Text("Дубли в лайках", color = C.text, fontSize = 18.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(horizontal = 20.dp))
            val g = groups
            if (g == null) Text("Ищем…", color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(20.dp))
            else if (g.isEmpty()) Text(note ?: "Дублей нет", color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(20.dp))
            else {
                Text("Найдено песен, лайкнутых больше раза: ${g.size}. Останется самый ранний лайк.", color = C.muted, fontSize = 13.sp, modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp))
                Column(Modifier.heightIn(max = 300.dp).verticalScroll(rememberScrollState())) {
                    g.forEach { grp -> Text("${grp.first().artist} - ${grp.first().title} (${grp.size})", color = C.text, fontSize = 14.sp, maxLines = 1, modifier = Modifier.padding(horizontal = 20.dp, vertical = 5.dp)) }
                }
                Text(
                    if (busy) "Убираем…" else "Убрать лишние", color = C.accent, fontSize = 15.sp, fontWeight = FontWeight.SemiBold,
                    modifier = Modifier.pressable {
                        if (busy) return@pressable
                        busy = true
                        scope.launch {
                            val extra = g.flatMap { it.drop(1) }
                            val gone = HashSet<Long>()
                            for (t in extra) {
                                if (runCatching { Writes.call(app, "like", JSONObject().put("track", t.id).put("on", false)) }.isSuccess) gone.add(t.id)
                                kotlinx.coroutines.delay(120)
                            }
                            onRemoved(gone)
                            android.widget.Toast.makeText(ctx, L.t("Готово"), android.widget.Toast.LENGTH_SHORT).show()
                            onClose()
                        }
                    }.padding(horizontal = 20.dp, vertical = 12.dp),
                )
            }
        }
    }
}

/** «Похожие треки»: what SoundCloud puts next to this song; a tap plays the list from that song. */
@Composable
internal fun SimilarDialog(app: App, t: Track, onClose: () -> Unit) {
    var list by remember { mutableStateOf<List<Track>?>(null) }
    var failed by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(t.id) {
        list = runCatching { withContext(kotlinx.coroutines.Dispatchers.IO) { app.api.related(t.id) } }.onFailure { failed = "Не получилось: ${it.message}" }.getOrDefault(emptyList())
    }
    androidx.compose.ui.window.Dialog(onDismissRequest = onClose, properties = androidx.compose.ui.window.DialogProperties(usePlatformDefaultWidth = false)) {
        Column(Modifier.padding(16.dp).clip(RoundedCornerShape(20.dp)).background(C.panel).padding(top = 16.dp)) {
            Text("Похожие треки", color = C.text, fontSize = 18.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(horizontal = 20.dp))
            Text(t.title, color = C.muted, fontSize = 13.sp, maxLines = 1, modifier = Modifier.padding(start = 20.dp, end = 20.dp, bottom = 8.dp))
            val l = list
            if (l == null) Text("Загружаем…", color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(20.dp))
            else if (l.isEmpty()) Text(failed ?: "Ничего не нашлось.", color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(20.dp))
            else androidx.compose.foundation.lazy.LazyColumn(Modifier.heightIn(max = 460.dp)) {
                itemsIndexed(l, key = { _, x -> x.id }) { i, x ->
                    TrackRow(x, false) { app.engine.playQueue(l.drop(i), 0); onClose() }
                }
            }
        }
    }
}

private fun count(n: Long): String = when {
    n >= 1_000_000 -> "%.1f млн".format(n / 1e6)
    n >= 10_000 -> "${n / 1000} тыс."
    else -> n.toString()
}

/** «Страница трека»: the cover, the plays and likes, the genre and the date, the text of the author, and what is similar. */
@Composable
internal fun TrackPageDialog(app: App, t: Track, onClose: () -> Unit) {
    val ctx = androidx.compose.ui.platform.LocalContext.current
    val scope = rememberCoroutineScope()
    var info by remember { mutableStateOf<JSONObject?>(null) }
    var similar by remember { mutableStateOf<List<Track>?>(null) }
    LaunchedEffect(t.id) {
        info = withContext(kotlinx.coroutines.Dispatchers.IO) { app.api.trackInfo(t.id) }
        similar = runCatching { withContext(kotlinx.coroutines.Dispatchers.IO) { app.api.related(t.id) } }.getOrDefault(emptyList())
    }
    androidx.compose.ui.window.Dialog(onDismissRequest = onClose, properties = androidx.compose.ui.window.DialogProperties(usePlatformDefaultWidth = false)) {
        Column(Modifier.padding(12.dp).clip(RoundedCornerShape(22.dp)).background(C.panel).verticalScroll(rememberScrollState()).padding(18.dp)) {
            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) { Cover(t.artwork?.replace("-t300x300", "-t500x500"), 220.dp, 18.dp) }
            Text(t.title, color = C.text, fontSize = 20.sp, fontWeight = FontWeight.Bold, modifier = Modifier.padding(top = 14.dp))
            Text(t.artist, color = C.muted, fontSize = 15.sp)
            val i = info
            if (i != null) {
                val bits = listOfNotNull(
                    i.optString("genre").takeIf { it.isNotEmpty() && it != "null" },
                    i.optString("created_at").take(4).takeIf { it.length == 4 && it.all { c -> c.isDigit() } },
                    t.durationMs.takeIf { it > 0 }?.let { "%d:%02d".format(it / 60000, it / 1000 % 60) },
                )
                if (bits.isNotEmpty()) Text(bits.joinToString(" · "), color = C.faint, fontSize = 13.sp, modifier = Modifier.padding(top = 2.dp))
                Text("▶ ${count(i.optLong("playback_count"))}   ♥ ${count(i.optLong("likes_count"))}   ↻ ${count(i.optLong("reposts_count"))}", color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(top = 8.dp))
                i.optString("description").takeIf { it.isNotBlank() && it != "null" }?.let { Text(it.take(500), color = C.muted, fontSize = 13.sp, modifier = Modifier.padding(top = 10.dp)) }
            } else Text("Загружаем…", color = C.faint, fontSize = 13.sp, modifier = Modifier.padding(top = 8.dp))
            Row(Modifier.padding(top = 14.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                Box(Modifier.weight(1f)) { BigButton("Слушать", true) { app.engine.playQueue(listOf(t), 0); onClose() } }
                Box(Modifier.weight(1f)) {
                    BigButton("Открыть на SoundCloud", false) {
                        val url = info?.optString("permalink_url")?.takeIf { it.startsWith("https://") }
                        if (url != null) ctx.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(url)).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK))
                    }
                }
            }
            Text("ПОХОЖИЕ ТРЕКИ", color = C.muted, fontSize = 11.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(top = 18.dp, bottom = 4.dp))
            val l = similar
            if (l == null) Text("Загружаем…", color = C.faint, fontSize = 13.sp)
            else if (l.isEmpty()) Text("Ничего не нашлось.", color = C.faint, fontSize = 13.sp)
            else l.take(15).forEachIndexed { k, x -> TrackRow(x, false) { app.engine.playQueue(l.drop(k), 0); onClose() } }
        }
    }
}
