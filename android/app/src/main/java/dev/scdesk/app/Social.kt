package dev.scdesk.app

import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.layout.size
import androidx.compose.ui.graphics.Color
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/** A track the computer sent: {id, title, artist, dur, art}. */
internal fun trackOf(j: JSONObject) = Track(j.optLong("id"), j.optString("title"), j.optString("artist"), j.optLong("dur"), j.optString("art").takeIf { it.isNotEmpty() && it != "null" })

private fun JSONArray?.objects(): List<JSONObject> = if (this == null) emptyList() else (0 until length()).map { getJSONObject(it) }

/** Asks the computer, shows a short wait and the failure; [content] draws the answer. */
@Composable
private fun <T> Asked(app: App, key: Any?, ask: suspend () -> T, content: @Composable (T, () -> Unit) -> Unit) {
    var tick by remember { mutableIntStateOf(0) }
    var value by remember(key) { mutableStateOf<T?>(null) }
    var failed by remember(key) { mutableStateOf<String?>(null) }
    LaunchedEffect(key, tick) {
        try {
            value = ask()
            failed = null
        } catch (e: Exception) {
            failed = if (!app.store.linked) "Сначала подключите компьютер: «Ещё», «Подключить компьютер»." else (e.message ?: "Не получилось") + ". Нужен включённый SC Desk на компьютере."
        }
    }
    failed?.let { Text(it, color = C.accent, fontSize = 14.sp, modifier = Modifier.padding(20.dp)) }
    val v = value
    if (v == null && failed == null) Text("Спрашиваем компьютер…", color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(20.dp))
    if (v != null) content(v) { tick++ }
}

@Composable
internal fun FriendsScreen(app: App, shown: Shown, play: (List<Track>, Int) -> Unit, onBack: () -> Unit) {
    val scope = rememberCoroutineScope()
    var find by remember { mutableStateOf("") }
    Page(onBack, "Друзья") {
        Asked(app, null, { app.link.rpc("friends").getJSONObject("data") }) { d, reload ->
            val friends = d.optJSONArray("friends").objects()
            val incoming = d.optJSONArray("incoming").objects()
            val listening = d.optJSONArray("listening").objects().associateBy { it.optLong("id") }
            LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 96.dp)) {
                if (incoming.isNotEmpty()) item { Text("ХОТЯТ ДРУЖИТЬ", color = C.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(start = 20.dp, top = 8.dp, bottom = 4.dp)) }
                itemsIndexed(incoming, key = { _, f -> "in${f.optLong("id")}" }) { _, f ->
                    Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 7.dp), verticalAlignment = Alignment.CenterVertically) {
                        Cover(f.optString("avatar").takeIf { it.isNotEmpty() && it != "null" }, 44.dp, 22.dp)
                        Text(f.optString("name"), color = C.text, fontSize = 16.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 12.dp).weight(1f))
                        Text("Принять", color = C.accent, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.pressable { scope.launch { runCatching { app.link.rpc("friend_accept", JSONObject().put("id", f.optLong("id"))) }; reload() } }.padding(8.dp))
                        Text("Нет", color = C.muted, fontSize = 14.sp, modifier = Modifier.pressable { scope.launch { runCatching { app.link.rpc("friend_decline", JSONObject().put("id", f.optLong("id"))) }; reload() } }.padding(8.dp))
                    }
                }
                item {
                    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).clip(RoundedCornerShape(14.dp)).background(C.panel).padding(horizontal = 14.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                        SIcon("search", C.muted)
                        Box(Modifier.padding(start = 10.dp).weight(1f)) {
                            if (find.isEmpty()) Text("Найти человека", color = C.faint, fontSize = 15.sp)
                            androidx.compose.foundation.text.BasicTextField(find, { find = it }, singleLine = true, textStyle = androidx.compose.ui.text.TextStyle(color = C.text, fontSize = 15.sp), cursorBrush = androidx.compose.ui.graphics.SolidColor(C.accent), modifier = Modifier.fillMaxWidth())
                        }
                    }
                }
                // people of SoundCloud who are not friends yet: a request goes to them (the computer sends it)
                item {
                    val found by produceState(emptyList<Artist>(), find) {
                        value = emptyList()
                        if (find.trim().length >= 2) {
                            kotlinx.coroutines.delay(500)
                            value = runCatching { withContext(Dispatchers.IO) { app.api.searchUsers(find.trim()) } }.getOrDefault(emptyList())
                        }
                    }
                    Column {
                        val fresh = found.filter { u -> friends.none { it.optLong("id") == u.id } }.take(5)
                        if (fresh.isNotEmpty()) Text("НАЙТИ ЛЮДЕЙ", color = C.muted, fontSize = 11.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(start = 20.dp, top = 8.dp, bottom = 2.dp))
                        fresh.forEach { u ->
                            var sent by remember(u.id) { mutableStateOf(false) }
                            Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                                Cover(u.avatar, 44.dp, 22.dp)
                                Text(u.name, color = C.text, fontSize = 16.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 12.dp).weight(1f))
                                Text(if (sent) "Отправлено" else "Добавить", color = C.accent, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.pressable {
                                    if (sent) return@pressable
                                    sent = true
                                    scope.launch { runCatching { app.link.rpc("friend_request", JSONObject().put("id", u.id).put("name", u.name).put("avatar", u.avatar ?: "")) }.onFailure { sent = false } }
                                }.padding(8.dp))
                            }
                        }
                    }
                }
                val hits = if (find.isBlank()) friends else friends.filter { it.optString("name").contains(find, true) }
                val now = hits.filter { listening.containsKey(it.optLong("id")) }
                val rest = hits.filter { !listening.containsKey(it.optLong("id")) }
                if (friends.isEmpty()) item { Text("Друзей пока нет. Друзей добавляют на компьютере, здесь они появятся сами.", color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(horizontal = 20.dp, vertical = 12.dp)) }
                if (now.isNotEmpty()) item { Text("СЕЙЧАС СЛУШАЮТ", color = C.muted, fontSize = 11.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(start = 20.dp, top = 12.dp, bottom = 4.dp)) }
                itemsIndexed(now, key = { _, f -> "n" + f.optLong("id") }) { _, f ->
                    val tr = listening[f.optLong("id")]?.optJSONObject("track")?.let { trackOf(it) }
                    Row(
                        Modifier.fillMaxWidth().pressable { tr?.let { t -> scope.launch { val l = withContext(Dispatchers.IO) { runCatching { app.api.tracksByIds(listOf(t.id)) }.getOrNull() }; if (!l.isNullOrEmpty()) play(l, 0) } } }.padding(horizontal = 20.dp, vertical = 7.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Cover(f.optString("avatar").takeIf { it.isNotEmpty() && it != "null" }, 48.dp, 24.dp)
                        Column(Modifier.padding(start = 14.dp).weight(1f)) {
                            Text(f.optString("name"), color = C.text, fontSize = 16.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text(tr?.let { it.title + " · " + it.artist } ?: "", color = C.accent, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                        SIcon("headphones", C.accent)
                    }
                }
                if (rest.isNotEmpty()) item { Text("ВСЕ ДРУЗЬЯ", color = C.muted, fontSize = 11.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(start = 20.dp, top = 16.dp, bottom = 4.dp)) }
                itemsIndexed(rest, key = { _, f -> "f" + f.optLong("id") }) { _, f ->
                    Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 7.dp), verticalAlignment = Alignment.CenterVertically) {
                        Cover(f.optString("avatar").takeIf { it.isNotEmpty() && it != "null" }, 48.dp, 24.dp)
                        Column(Modifier.padding(start = 14.dp).weight(1f)) {
                            Text(f.optString("name"), color = C.text, fontSize = 16.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text("не слушает сейчас", color = C.faint, fontSize = 12.sp)
                        }
                        Text("Удалить", color = C.muted, fontSize = 13.sp, modifier = Modifier.pressable { scope.launch { runCatching { app.link.rpc("friend_remove", JSONObject().put("id", f.optLong("id"))) }; reload() } }.padding(8.dp))
                    }
                }
            }
        }
    }
}

@Composable
internal fun SharedScreen(app: App, shown: Shown, play: (List<Track>, Int) -> Unit, onBack: () -> Unit) {
    val scope = rememberCoroutineScope()
    var open by remember { mutableStateOf<JSONObject?>(null) }
    val ctx = androidx.compose.ui.platform.LocalContext.current
    var note by remember { mutableStateOf<String?>(null) }
    var actions by remember { mutableStateOf<JSONObject?>(null) }
    var newName by remember { mutableStateOf("") }
    val o = open
    if (o != null) {
        SharedList(app, shown, play, o) { open = null }
        return
    }
    Page(onBack, "Общие плейлисты") {
        Asked(app, note, { app.link.rpc("sp_list").getJSONArray("data").objects() }) { list, reload ->
            Box(Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) {
                BigButton("Вступить по ссылке из буфера", false) {
                    val link = (ctx.getSystemService(android.content.ClipboardManager::class.java).primaryClip?.getItemAt(0)?.text ?: "").toString().trim()
                    scope.launch {
                        note = try {
                            if (link.isEmpty()) "В буфере нет ссылки" else { app.link.rpc("sp_join", JSONObject().put("link", link)); reload(); "Вы в плейлисте" }
                        } catch (e: Exception) { e.message }
                    }
                }
            }
            // a new shared playlist: a name, and the computer makes it (the others join by the link)
            Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp).clip(RoundedCornerShape(14.dp)).background(C.panel).padding(horizontal = 14.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.weight(1f)) {
                    if (newName.isEmpty()) Text("Новый общий плейлист", color = C.faint, fontSize = 15.sp)
                    androidx.compose.foundation.text.BasicTextField(newName, { newName = it }, singleLine = true, textStyle = androidx.compose.ui.text.TextStyle(color = C.text, fontSize = 15.sp), cursorBrush = androidx.compose.ui.graphics.SolidColor(C.accent), modifier = Modifier.fillMaxWidth())
                }
                Text("Создать", color = C.accent, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.pressable {
                    if (newName.isBlank()) return@pressable
                    val name = newName.trim()
                    scope.launch { note = try { app.link.rpc("sp_create", JSONObject().put("title", name)); newName = ""; reload(); "Создан: " + name } catch (e: Exception) { e.message } }
                }.padding(8.dp))
            }
            actions?.let { p ->
                var title by remember(p) { mutableStateOf(p.optString("title")) }
                androidx.compose.ui.window.Dialog(onDismissRequest = { actions = null }) {
                    Column(Modifier.clip(RoundedCornerShape(20.dp)).background(C.panel).padding(20.dp).heightIn(max = 560.dp).verticalScroll(rememberScrollState())) {
                        Text("Общий плейлист", color = C.text, fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
                        Box(Modifier.padding(top = 12.dp).fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(C.panel2).padding(horizontal = 14.dp, vertical = 12.dp)) {
                            androidx.compose.foundation.text.BasicTextField(title, { title = it }, singleLine = true, textStyle = androidx.compose.ui.text.TextStyle(color = C.text, fontSize = 15.sp), cursorBrush = androidx.compose.ui.graphics.SolidColor(C.accent), modifier = Modifier.fillMaxWidth())
                        }
                        Text("Переименовать", color = C.accent, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.pressable {
                            val ch = p.optString("channel")
                            scope.launch { note = try { app.link.rpc("sp_title", JSONObject().put("channel", ch).put("title", title.trim())); actions = null; reload(); "Переименован" } catch (e: Exception) { e.message } }
                        }.padding(vertical = 12.dp))
                        if (p.optBoolean("mine")) {
                            val ch = p.optString("channel")
                            val modes = listOf("closed" to "Закрыт", "invite" to "По приглашению", "link" to "По ссылке", "open" to "Открыт")
                            var access by remember(p) { mutableStateOf(p.optString("access").ifEmpty { "link" }) }
                            Text("ДОСТУП", color = C.muted, fontSize = 11.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(top = 10.dp, bottom = 6.dp))
                            Segments(modes.map { it.second }, modes.indexOfFirst { it.first == access }.coerceAtLeast(0)) { i ->
                                access = modes[i].first
                                scope.launch { runCatching { app.link.rpc("sp_access", JSONObject().put("channel", ch).put("access", access)) }.onFailure { note = it.message } }
                            }
                            Text("ОФОРМЛЕНИЕ", color = C.muted, fontSize = 11.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(top = 12.dp, bottom = 6.dp))
                            var color by remember(p) { mutableStateOf(p.optString("color").takeIf { it != "null" && it.isNotEmpty() }) }
                            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                                listOf("#ff7b3f", "#ec5b7a", "#b77dee", "#5b9df5", "#3fbf9a", "#f5c242", "#2c2828").forEach { hex ->
                                    Box(Modifier.size(30.dp).clip(androidx.compose.foundation.shape.CircleShape).background(Color(android.graphics.Color.parseColor(hex))).border(if (color == hex) 3.dp else 0.dp, C.text, androidx.compose.foundation.shape.CircleShape).pressable {
                                        color = hex
                                        scope.launch { runCatching { app.link.rpc("sp_cover", JSONObject().put("channel", ch).put("color", hex)) }.onFailure { note = it.message } }
                                    })
                                }
                            }
                            Text("ПРИГЛАСИТЬ", color = C.muted, fontSize = 11.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(top = 12.dp, bottom = 4.dp))
                            val friends by produceState(emptyList<JSONObject>()) { value = runCatching { app.link.rpc("friends").getJSONObject("data").optJSONArray("friends").objects() }.getOrDefault(emptyList()) }
                            var viewerOnly by remember(p) { mutableStateOf(false) }
                            Text(if (viewerOnly) "Только слушать" else "Может добавлять песни", color = C.accent, fontSize = 13.sp, modifier = Modifier.pressable { viewerOnly = !viewerOnly }.padding(vertical = 6.dp))
                            if (friends.isEmpty()) Text("Друзей пока нет.", color = C.faint, fontSize = 13.sp)
                            friends.take(20).forEach { f ->
                                var sent by remember(f.optLong("id")) { mutableStateOf(false) }
                                Row(Modifier.fillMaxWidth().padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                                    Text(f.optString("name"), color = C.text, fontSize = 15.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                                    Text(if (sent) "Отправлено" else "Пригласить", color = C.accent, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.pressable {
                                        if (sent) return@pressable
                                        sent = true
                                        scope.launch { runCatching { app.link.rpc("sp_invite", JSONObject().put("channel", ch).put("user_id", f.optLong("id")).put("name", f.optString("name")).put("role", if (viewerOnly) "viewer" else "editor")) }.onFailure { sent = false; note = it.message } }
                                    }.padding(8.dp))
                                }
                            }
                        }
                        Text("Покинуть плейлист", color = C.muted, fontSize = 15.sp, modifier = Modifier.pressable {
                            val ch = p.optString("channel")
                            scope.launch { note = try { app.link.rpc("sp_leave", JSONObject().put("channel", ch)); actions = null; reload(); "Вы вышли" } catch (e: Exception) { e.message } }
                        }.padding(vertical = 8.dp))
                    }
                }
            }
            note?.let { Text(it, color = C.muted, fontSize = 13.sp, modifier = Modifier.padding(horizontal = 20.dp, vertical = 4.dp)) }
            if (list.isEmpty()) Text("Общих плейлистов пока нет. Создать или принять приглашение можно на компьютере, здесь они появятся.", color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(20.dp))
            LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 96.dp)) {
                itemsIndexed(list, key = { _, p -> p.optString("channel") }) { _, p ->
                    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 5.dp).clip(RoundedCornerShape(16.dp)).background(C.panel).pressableLong({ open = p }, { actions = p }).padding(14.dp), verticalAlignment = Alignment.CenterVertically) {
                        Cover(p.optString("art").takeIf { it.isNotEmpty() && it != "null" }, 64.dp, 12.dp)
                        Column(Modifier.padding(start = 14.dp).weight(1f)) {
                            Text(p.optString("title"), color = C.text, fontSize = 17.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text("${p.optInt("count")} треков" + if (p.optBoolean("can_edit")) "" else " · только слушать", color = C.muted, fontSize = 13.sp)
                        }
                        SIcon("chev", C.muted)
                    }
                }
            }
        }
    }
}

@Composable
private fun SharedList(app: App, shown: Shown, play: (List<Track>, Int) -> Unit, p: JSONObject, onBack: () -> Unit) {
    val scope = rememberCoroutineScope()
    var note by remember { mutableStateOf<String?>(null) }
    Page(onBack, p.optString("title")) {
        Asked(app, note, { app.link.rpc("sp_get", JSONObject().put("channel", p.optString("channel"))).getJSONObject("data") }) { d, reload ->
            val tracks = d.optJSONArray("tracks").objects()
            val list = tracks.map { trackOf(it) }
            val cur = shown.track
            Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Box(Modifier.weight(1f)) { BigButton("Слушать", true) { if (list.isNotEmpty()) play(list, 0) } }
                if (d.optBoolean("can_edit") && cur != null) Box(Modifier.weight(1f)) {
                    BigButton("Добавить играющий", false) {
                        scope.launch { note = try { app.link.rpc("sp_add", JSONObject().put("channel", p.optString("channel")).put("track", cur.id)); reload(); "Добавлено" } catch (e: Exception) { e.message } }
                    }
                }
            }
            note?.let { Text(it, color = C.muted, fontSize = 13.sp, modifier = Modifier.padding(horizontal = 20.dp, vertical = 4.dp)) }
            LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 96.dp)) {
                itemsIndexed(list, key = { i, t -> "$i-${t.id}" }) { i, t -> TrackRow(t, shown.track?.id == t.id) { play(list, i) } }
            }
        }
    }
}
