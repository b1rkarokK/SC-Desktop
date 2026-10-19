package dev.scdesk.app

import androidx.compose.foundation.background
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
        if (!app.store.linked) { note = "Нужен подключённый компьютер: плейлисты меняет он."; lists = emptyList(); return@LaunchedEffect }
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
                                runCatching { app.link.rpc("pl_create", JSONObject().put("title", title.trim()).put("track", track.id)) }
                                    .onSuccess { toast("Плейлист «${title.trim()}» создан"); onClose() }
                                    .onFailure { note = "Не вышло: ${it.message}"; busy = false }
                            }
                        }.padding(vertical = 10.dp))
                    }
                } else if (app.store.linked && !shared) {
                    Text("＋ Новый плейлист", color = C.accent, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.fillMaxWidth().pressable { creating = true }.padding(horizontal = 20.dp, vertical = 12.dp))
                }
                when (val l = lists) {
                    null -> Text("Загружаем…", color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(20.dp))
                    else -> l.forEach { p ->
                        Row(Modifier.fillMaxWidth().pressable {
                            if (busy) return@pressable
                            busy = true
                            scope.launch {
                                runCatching { if (shared) app.link.rpc("sp_add", JSONObject().put("channel", p.channel).put("track", track.id)) else app.link.rpc("pl_add", JSONObject().put("playlist", p.id).put("track", track.id)) }
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
                        runCatching { app.link.rpc("pl_delete", JSONObject().put("playlist", id)) }
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
internal fun TrackActions(app: App, t: Track, onComputer: Boolean, fromPlaylist: Long? = null, onRemoved: () -> Unit = {}, onClose: () -> Unit) {
    val scope = rememberCoroutineScope()
    val ctx = androidx.compose.ui.platform.LocalContext.current
    var picker by remember { mutableStateOf(0) } // 1 own playlists, 2 shared ones
    fun toast(s: String) = android.widget.Toast.makeText(ctx, L.t(s), android.widget.Toast.LENGTH_SHORT).show()
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
        "Добавить в плейлист…" to { picker = 1 },
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
                runCatching { app.link.rpc("pl_remove", JSONObject().put("playlist", fromPlaylist ?: 0L).put("track", t.id)) }
                    .onSuccess { onRemoved(); toast("Убрано из плейлиста") }
                    .onFailure { android.widget.Toast.makeText(ctx, L.t("Не получилось: {0}").replace("{0}", it.message ?: ""), android.widget.Toast.LENGTH_LONG).show() }
            }
            onClose()
        },
        "Убрать из лайков" to {
            scope.launch { runCatching { app.link.rpc("like", JSONObject().put("track", t.id).put("on", false)) } }
            toast("Убрано из лайков"); onClose()
        },
    )
    androidx.compose.ui.window.Dialog(onDismissRequest = onClose) {
        Column(Modifier.clip(RoundedCornerShape(20.dp)).background(C.panel).padding(vertical = 14.dp)) {
            Text(t.title, color = C.text, fontSize = 16.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, modifier = Modifier.padding(horizontal = 20.dp))
            Text(t.artist, color = C.muted, fontSize = 13.sp, maxLines = 1, modifier = Modifier.padding(start = 20.dp, end = 20.dp, bottom = 8.dp))
            Column(Modifier.heightIn(max = 420.dp).verticalScroll(rememberScrollState())) {
                items.filter { it.first != "Убрать из этого плейлиста" || (fromPlaylist != null && app.store.linked) }.forEach { (label, act) -> Text(label, color = C.text, fontSize = 15.sp, modifier = Modifier.fillMaxWidth().pressable(act).padding(horizontal = 20.dp, vertical = 13.dp)) }
            }
        }
    }
}
