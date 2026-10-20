@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)

package dev.scdesk.app

import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
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
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** A list of tracks opened over the tabs (a mix, a playlist, the uploads of an artist). */
data class Opened(val title: String, val artist: Artist? = null, val playlistId: Long? = null, val load: () -> List<Track>)

@Composable
internal fun Chip(text: String, on: Boolean, onClick: () -> Unit) {
    val shape = RoundedCornerShape(18.dp)
    Text(
        text, color = if (on) C.accent else C.muted, fontSize = 14.sp, fontWeight = if (on) FontWeight.SemiBold else FontWeight.Normal,
        modifier = Modifier.clip(shape).background(if (on) C.warm else C.panel).border(1.dp, if (on) C.accent else C.border, shape).pressable(onClick).padding(horizontal = 16.dp, vertical = 8.dp),
    )
}

@Composable
private fun GuestPrompt(title: String, text: String, onLogin: () -> Unit) {
    Column(Modifier.fillMaxSize().padding(24.dp), verticalArrangement = Arrangement.Center, horizontalAlignment = Alignment.CenterHorizontally) {
        Text(title, color = C.text, fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
        Text(text, color = C.muted, fontSize = 14.sp, textAlign = TextAlign.Center, modifier = Modifier.padding(vertical = 10.dp))
        BigButton("Войти через SoundCloud", true, onLogin)
    }
}

@Composable
private fun SectionTitle(text: String) {
    Text(text, color = C.text, fontSize = 18.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(start = 20.dp, top = 22.dp, bottom = 10.dp))
}

// -------------------------------------------------------------------- home

@Composable
internal fun HomeTab(app: App, shown: Shown, account: Int, play: (List<Track>, Int) -> Unit, open: (Opened) -> Unit, onLogin: () -> Unit, onPage: (String) -> Unit = {}) {
    val scope = rememberCoroutineScope()
    var shelves by remember(account) { mutableStateOf<List<Shelf>?>(null) }
    var fresh by remember(account) { mutableStateOf(listOf<Track>()) }
    var failed by remember(account) { mutableStateOf<String?>(null) }
    val signedIn = app.store.signedIn
    // the friends who listen now (asked of the computer, once; nothing is shown when it is away)
    var friends by remember(account) { mutableStateOf<List<org.json.JSONObject>>(emptyList()) }
    var friendNames by remember(account) { mutableStateOf<Map<Long, String>>(emptyMap()) }
    LaunchedEffect(account) {
        if (!app.store.linked) return@LaunchedEffect
        runCatching {
            val d = app.link.rpc("friends").getJSONObject("data")
            val ls = d.optJSONArray("listening")
            friends = if (ls == null) emptyList() else (0 until ls.length()).map { ls.getJSONObject(it) }
            val fs = d.optJSONArray("friends")
            friendNames = if (fs == null) emptyMap() else (0 until fs.length()).associate { fs.getJSONObject(it).optLong("id") to fs.getJSONObject(it).optString("name") }
        }
    }
    // 0 the shelves, 1 the categories, 2 the feed of the people followed
    var inner by remember { mutableIntStateOf(0) }
    LaunchedEffect(account) {
        if (!signedIn) return@LaunchedEffect
        scope.launch {
            try {
                // what was seen last time shows at once, the fresh one replaces it when it comes
                runCatching { withContext(Dispatchers.IO) { app.api.stale { app.api.home() } } }.getOrNull()?.let { shelves = it }
                shelves = withContext(Dispatchers.IO) { app.api.home() }
                fresh = withContext(Dispatchers.IO) { app.api.fromFollowings() }
                failed = null
            } catch (e: Exception) {
                failed = "Не получилось загрузить главную: ${e.message}"
            }
        }
    }
    Column(Modifier.fillMaxSize()) {
        Head("Главная") { Box(Modifier.size(44.dp).pressable { onPage("friends") }, contentAlignment = Alignment.Center) { SIcon("bell", C.text) } }
        Box(Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) { Segments(listOf("Главная", "Категории", "Лента"), inner) { inner = it } }
        if (inner == 1) {
            CategoriesList(app, open)
            return@Column
        }
        if (!signedIn) {
            GuestPrompt("Подборки для вас", "Миксы и новое от тех, на кого вы подписаны, появятся после входа в SoundCloud.", onLogin)
            return@Column
        }
        if (inner == 2) {
            failed?.let { Text(it, color = C.accent, fontSize = 14.sp, modifier = Modifier.padding(20.dp)) }
            if (fresh.isEmpty() && failed == null) Text("Загружаем ленту…", color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(20.dp))
            LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 96.dp)) {
                itemsIndexed(fresh, key = { _, t -> "feed" + t.id }) { i, t -> TrackRow(t, shown.track?.id == t.id) { play(fresh, i) } }
            }
            return@Column
        }
        failed?.let { Text(it, color = C.accent, fontSize = 14.sp, modifier = Modifier.padding(20.dp)) }
        if (shelves == null && failed == null) Text("Загружаем…", color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(20.dp))
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 12.dp)) {
            items(shelves ?: emptyList(), key = { it.title }) { shelf ->
                SectionTitle(shelf.title)
                LazyRow(contentPadding = PaddingValues(horizontal = 20.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    items(shelf.cards, key = { (it.urn ?: "p${it.id}") }) { c ->
                        Column(Modifier.width(132.dp).pressable { open(Opened(c.title) { if (c.kind == "mix") app.api.mixTracks(c.urn!!) else app.api.playlistTracks(c.id) }) }) {
                            Cover(c.artwork, 132.dp, 14.dp)
                            Text(c.title, color = C.text, fontSize = 14.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(top = 6.dp))
                            Text(c.subtitle, color = C.muted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                    }
                }
            }
            if (friends.isNotEmpty()) {
                item { SectionTitle("Друзья слушают") }
                items(friends.size, key = { "fl" + friends[it].optLong("id") }) { k ->
                    val f = friends[k]
                    val tr = f.optJSONObject("track")?.let { trackOf(it) }
                    val name = friendNames[f.optLong("id")] ?: "Друг"
                    Row(
                        Modifier.fillMaxWidth().pressable { tr?.let { t -> scope.launch { val l = withContext(Dispatchers.IO) { runCatching { app.api.tracksByIds(listOf(t.id)) }.getOrNull() }; if (!l.isNullOrEmpty()) play(l, 0) } } }.padding(horizontal = 20.dp, vertical = 7.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Box(Modifier.size(44.dp).clip(androidx.compose.foundation.shape.CircleShape).background(C.panel2), contentAlignment = Alignment.Center) { Text(name.take(1).uppercase(), color = C.text, fontSize = 17.sp, fontWeight = FontWeight.SemiBold) }
                        Column(Modifier.padding(start = 12.dp).weight(1f)) {
                            Text(name, color = C.text, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
                            Text(tr?.let { it.title + " · " + it.artist } ?: "", color = C.muted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                        SIcon("headphones", C.accent)
                    }
                }
            }
            if (fresh.isNotEmpty()) {
                item { SectionTitle("Новое от подписок") }
                itemsIndexed(fresh, key = { _, t -> "f${t.id}" }) { i, t -> TrackRow(t, shown.track?.id == t.id) { play(fresh, i) } }
            }
        }
    }
}

// ------------------------------------------------------------------- likes

@Composable
internal fun LikesTab(app: App, shown: Shown, account: Int, play: (List<Track>, Int) -> Unit, open: (Opened) -> Unit, onLogin: () -> Unit, onDevices: () -> Unit, remote: Boolean) {
    val scope = rememberCoroutineScope()
    var inner by remember { mutableStateOf(0) }
    var searching by remember { mutableStateOf(false) }
    var filter by remember { mutableStateOf("") }
    var tracks by remember(account) { mutableStateOf(listOf<Track>()) }
    var next by remember(account) { mutableStateOf<String?>(null) }
    var loading by remember(account) { mutableStateOf(false) }
    var failed by remember(account) { mutableStateOf<String?>(null) }
    var playlists by remember(account) { mutableStateOf<List<Card>?>(null) }
    var artists by remember(account) { mutableStateOf<List<Artist>?>(null) }
    val signedIn = app.store.signedIn
    var plTick by remember { mutableIntStateOf(0) }
    var dupsOpen by remember { mutableStateOf(false) }
    if (dupsOpen) DuplicatesDialog(app, onClose = { dupsOpen = false }) { removed -> tracks = tracks.filter { x -> x.id !in removed } }
    var actions by remember { mutableStateOf<Track?>(null) }
    actions?.let { tr -> TrackActions(app, tr, remote, onRemovedLike = { tracks = tracks.filter { x -> x.id != tr.id } }) { actions = null } }

    fun more(first: Boolean = false) {
        if (loading || (!first && next == null)) return
        loading = true
        scope.launch {
            try {
                if (first && tracks.isEmpty()) runCatching { withContext(Dispatchers.IO) { app.api.stale { app.api.likes(null) } } }.getOrNull()?.let { tracks = it.first; next = it.second }
                val (page, n) = withContext(Dispatchers.IO) { app.api.likes(if (first) null else next) }
                tracks = if (first) page else tracks + page
                // what this list shows is liked: the hearts of the player know it
                app.liked.value = app.liked.value + page.map { it.id }
                next = n
                failed = null
            } catch (e: Exception) {
                failed = "Не получилось загрузить лайки: ${e.message}"
            }
            loading = false
        }
    }
    LaunchedEffect(account) { if (signedIn) more(true) }
    LaunchedEffect(inner, account, plTick) {
        if (!signedIn) return@LaunchedEffect
        try {
            if (inner == 1 && playlists == null) playlists = withContext(Dispatchers.IO) { app.api.likedPlaylists() }
            if (inner == 2 && artists == null) artists = withContext(Dispatchers.IO) { app.api.followings() }
        } catch (e: Exception) {
            failed = "Не получилось загрузить: ${e.message}"
        }
    }
    val list = rememberLazyListState()
    LaunchedEffect(list, account) {
        snapshotFlow { list.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0 }.collect { last -> if (inner == 0 && last >= tracks.size - 8 && tracks.isNotEmpty()) more() }
    }

    Column(Modifier.fillMaxSize()) {
        Head("Лайки") {
            Row {
                Box(Modifier.size(44.dp).pressable { searching = !searching; if (!searching) filter = "" }, contentAlignment = Alignment.Center) { SIcon("search", if (searching) C.accent else C.text) }
                Box(Modifier.size(44.dp).pressable { app.api.forget(); more(true) }, contentAlignment = Alignment.Center) { SIcon("refresh", C.text) }
            }
        }
        if (!signedIn) {
            GuestPrompt("Здесь будут ваши лайки", "Войдите в SoundCloud, и вся ваша музыка окажется в телефоне.", onLogin)
            return@Column
        }
        Box(Modifier.padding(horizontal = 16.dp, vertical = 6.dp)) { Segments(listOf("Треки", "Плейлисты", "Артисты"), inner) { inner = it } }
        if (searching) Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp).clip(RoundedCornerShape(14.dp)).background(C.panel).padding(horizontal = 14.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            SIcon("search", C.muted)
            Box(Modifier.padding(start = 10.dp).weight(1f)) {
                if (filter.isEmpty()) Text("Название или исполнитель", color = C.faint, fontSize = 15.sp)
                androidx.compose.foundation.text.BasicTextField(filter, { filter = it }, singleLine = true, textStyle = androidx.compose.ui.text.TextStyle(color = C.text, fontSize = 15.sp), cursorBrush = androidx.compose.ui.graphics.SolidColor(C.accent), modifier = Modifier.fillMaxWidth())
            }
        }
        val visible = if (filter.isBlank()) tracks else tracks.filter { it.title.contains(filter, true) || it.artist.contains(filter, true) }
        failed?.let { Text(it, color = C.accent, fontSize = 14.sp, modifier = Modifier.padding(20.dp)) }
        LazyColumn(Modifier.fillMaxSize(), state = list, contentPadding = PaddingValues(bottom = 12.dp)) {
            when (inner) {
                0 -> {
                    item { PendingLikes(app, account) }
                    if (tracks.isNotEmpty()) item {
                        val ctx = androidx.compose.ui.platform.LocalContext.current
                        val batch by Downloads.batch.collectAsState()
                        Column(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 4.dp)) {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Text("${tracks.size}${if (next != null) "+" else ""} треков", color = C.muted, fontSize = 12.sp, modifier = Modifier.weight(1f))
                                val b = batch
                                Text("Дубли", color = C.accent, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.pressable { dupsOpen = true }.padding(6.dp))
                                if (b?.running == true) Text("Остановить", color = C.accent, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.pressable { Downloads.stopAll() }.padding(6.dp))
                                else Text("Скачать все", color = C.accent, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.pressable {
                                    // all the likes (not only the pages shown), saved one after another off the screen
                                    app.appScope.launch {
                                        Downloads.batch.value = Downloads.Batch(0, 0, 0, true, "Собираем список лайков…")
                                        val all = runCatching { app.api.likesDated(5000).map { it.first } }.getOrDefault(tracks)
                                        Downloads.saveAll(ctx, app, all)
                                    }
                                }.padding(6.dp))
                            }
                            batch?.let { b ->
                                if (b.running || b.note != null || b.total > 0) {
                                    Text(
                                        b.note ?: (if (b.running) "Скачивается ${b.done} из ${b.total}" else "Скачано ${b.done - b.failed} из ${b.total}" + if (b.failed > 0) ", не вышло: ${b.failed}" else ""),
                                        color = C.muted, fontSize = 12.sp, modifier = Modifier.padding(top = 2.dp).pressable { Downloads.dismissBatch() },
                                    )
                                    if (b.total > 0) Box(Modifier.padding(top = 6.dp).fillMaxWidth().height(4.dp).clip(RoundedCornerShape(2.dp)).background(C.panel2)) {
                                        Box(Modifier.fillMaxWidth((b.done.toFloat() / b.total).coerceIn(0f, 1f)).height(4.dp).background(C.accent))
                                    }
                                }
                            }
                        }
                    }
                    itemsIndexed(visible, key = { _, t -> t.id }) { i, t ->
                        TrackRow(t, shown.track?.id == t.id, onLong = { actions = t }, trailing = {
                            if (app.rooms.state.value != null) Box(Modifier.size(40.dp).pressable {
                                // in a room the plus puts the track into its queue
                                scope.launch { runCatching { app.link.rpc("room_enqueue", org.json.JSONObject().put("track", t.id)) } }
                            }, contentAlignment = Alignment.Center) { SIcon("plus", C.text) }
                            else Box(Modifier.size(40.dp).pressable {
                                // the heart takes the like off (the computer does it: the writes need its browser session)
                                tracks = tracks - t
                                scope.launch { runCatching { Writes.call(app, "like", org.json.JSONObject().put("track", t.id).put("on", false)) }.onFailure { tracks = listOf(t) + tracks } }
                            }, contentAlignment = Alignment.Center) { Icon(Ico.Heart, 22.dp, C.accent) }
                        }) { play(visible, i) }
                    }
                }
                1 -> { item { NewPlaylist(app) { playlists = null; plTick++ } }; item { ImportPlaylist(app) }; items(playlists ?: emptyList(), key = { it.id }) { c ->
                    var askDelete by remember { mutableStateOf(false) }
                    val ctxP = androidx.compose.ui.platform.LocalContext.current
                    if (askDelete) DeletePlaylist(app, c.id, c.title, onClose = { askDelete = false }) { playlists = playlists?.filter { it.id != c.id } }
                    Row(Modifier.fillMaxWidth().combinedClickable(onClick = { open(Opened(c.title, null, c.id) { app.api.playlistTracks(c.id) }) }, onLongClick = { if (app.store.linked) askDelete = true else android.widget.Toast.makeText(ctxP, L.t("Нужен подключённый компьютер: плейлисты меняет он."), android.widget.Toast.LENGTH_SHORT).show() }).padding(horizontal = 20.dp, vertical = 7.dp), verticalAlignment = Alignment.CenterVertically) {
                        Cover(c.artwork, 56.dp, 10.dp)
                        Column(Modifier.padding(start = 14.dp).weight(1f)) {
                            Text(c.title, color = C.text, fontSize = 16.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text(c.subtitle, color = C.muted, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                    }
                } }
                else -> items(artists ?: emptyList(), key = { it.id }) { a ->
                    Row(Modifier.fillMaxWidth().pressable { open(Opened(a.name, a) { app.api.artistTracks(a.id) }) }.padding(horizontal = 20.dp, vertical = 7.dp), verticalAlignment = Alignment.CenterVertically) {
                        Cover(a.avatar, 48.dp, 24.dp)
                        Text(a.name, color = C.text, fontSize = 16.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 14.dp))
                    }
                }
            }
        }
    }
}

// -------------------------------------------------------------------- wave

/** «Моя волна»: the card of what plays, the mood and the settings as on the computer, the next tracks, the exclusions. */
@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
internal fun WaveTab(app: App, shown: Shown, waveList: List<Track>, vibe: String, busy: Boolean, onVibe: (String) -> Unit, onStart: () -> Unit, onLogin: () -> Unit, play: (List<Track>, Int) -> Unit, onDislike: (Track) -> Unit = {}) {
    val wave = app.wave
    Column(Modifier.fillMaxSize()) {
        Head("Моя волна")
        if (!app.store.signedIn) {
            GuestPrompt("Волна по вашему вкусу", "Волна собирается из ваших лайков. Войдите в SoundCloud, чтобы её запустить.", onLogin)
            return@Column
        }
        val cur = if (wave.active) shown.track else null
        val after = waveList.dropWhile { it.id != shown.track?.id }.drop(1).take(15)
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 96.dp)) {
            item {
                Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).clip(RoundedCornerShape(18.dp)).background(C.panel).border(1.dp, C.border, RoundedCornerShape(18.dp)).padding(14.dp), verticalAlignment = Alignment.CenterVertically) {
                    Cover(cur?.artwork, 76.dp, 12.dp)
                    Column(Modifier.padding(start = 14.dp).weight(1f)) {
                        Text(cur?.title ?: "Волна не запущена", color = C.text, fontSize = 17.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(cur?.artist ?: "Подбор из ваших лайков и похожих треков", color = C.muted, fontSize = 13.sp, maxLines = 2, overflow = TextOverflow.Ellipsis)
                        cur?.let {
                            Row(Modifier.padding(top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                                Box(Modifier.size(6.dp).clip(androidx.compose.foundation.shape.CircleShape).background(C.accent))
                                Text(wave.reasons[it.id] ?: "", color = C.accent, fontSize = 12.sp, modifier = Modifier.padding(start = 6.dp), maxLines = 1, overflow = TextOverflow.Ellipsis)
                            }
                        }
                    }
                    Box(Modifier.padding(start = 10.dp).size(48.dp).clip(if (C.squareButton) RoundedCornerShape(16.dp) else androidx.compose.foundation.shape.CircleShape).background(C.text).pressable { if (cur != null) app.engine.toggle() else if (!busy) onStart() }, contentAlignment = Alignment.Center) {
                        Icon(if (cur != null && shown.playing) Ico.Pause else Ico.Play, 28.dp, C.bg)
                    }
                }
            }
            item { Box(Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) { BigButton(if (busy) "Подбираем…" else if (wave.active) "Обновить волну" else "Запустить волну", true) { if (!busy) onStart() } } }
            if (cur != null) item {
                Row(Modifier.padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("Не нравится", color = C.text, fontSize = 13.sp, fontWeight = FontWeight.Medium, modifier = Modifier.clip(RoundedCornerShape(14.dp)).background(C.panel).pressable { onDislike(cur) }.padding(horizontal = 14.dp, vertical = 10.dp))
                    Text("Не ставить артиста", color = C.text, fontSize = 13.sp, fontWeight = FontWeight.Medium, modifier = Modifier.clip(RoundedCornerShape(14.dp)).background(C.panel).pressable { wave.banArtist(cur); onDislike(cur) }.padding(horizontal = 14.dp, vertical = 10.dp))
                }
            }
            item { WaveLabel("НАСТРОЕНИЕ") }
            item {
                androidx.compose.foundation.layout.FlowRow(Modifier.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Vibes.all.forEach { (id, label) -> Chip(if (id == "any") "Как обычно" else label, vibe == id) { onVibe(id) } }
                }
            }
            item { WaveLabel("НОВИЗНА") }
            item {
                Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    listOf("Как обычно", "Больше нового", "Только знакомое").forEachIndexed { i, l -> Chip(l, wave.novelty == i) { wave.novelty = i } }
                }
            }
            item { WaveLabel("ЯЗЫК ПЕСЕН") }
            item {
                Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    listOf("Любой язык", "Только на русском", "Только иностранное").forEachIndexed { i, l -> Chip(l, wave.language == i) { wave.language = i } }
                }
            }
            item { Toggle("Без лайкнутых", "не предлагать то, что уже в лайках", wave.noLiked) { wave.noLiked = it } }
            item { Toggle("Без слушанного", "только то, что вы ещё не включали на телефоне", wave.noHeard) { wave.noHeard = it } }
            if (wave.active && after.isNotEmpty()) {
                item { WaveLabel("ДАЛЬШЕ В ВОЛНЕ") }
                itemsIndexed(after, key = { _, t -> "w" + t.id }) { _, t ->
                    Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                        Cover(t.artwork, 44.dp, 8.dp)
                        Column(Modifier.padding(start = 12.dp).weight(1f)) {
                            Text(t.title, color = C.text, fontSize = 15.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text(wave.reasons[t.id] ?: t.artist, color = C.muted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                        Box(Modifier.size(40.dp).pressable { onDislike(t) }, contentAlignment = Alignment.Center) { SIcon("dislike", C.faint) }
                    }
                }
            }
            item {
                Row(Modifier.fillMaxWidth().padding(end = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Box(Modifier.weight(1f)) { WaveLabel("ИСКЛЮЧЁННЫЕ АРТИСТЫ") }
                    if (wave.bannedArtists.isNotEmpty() || wave.dislikedList.isNotEmpty()) Text("Сбросить всё", color = C.accent, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.pressable { wave.resetBans() }.padding(8.dp))
                }
            }
            if (wave.bannedArtists.isEmpty()) item { Text("Артистов в исключениях нет.", color = C.faint, fontSize = 13.sp, modifier = Modifier.padding(horizontal = 20.dp)) }
            items(wave.bannedArtists.entries.toList(), key = { "b" + it.key }) { e ->
                Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(e.value, color = C.text, fontSize = 15.sp, modifier = Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text("Вернуть", color = C.accent, fontSize = 13.sp, modifier = Modifier.pressable { wave.unbanArtist(e.key) }.padding(8.dp))
                }
            }
            item { WaveLabel("НЕ РЕКОМЕНДОВАТЬ (ТРЕКИ)") }
            if (wave.dislikedList.isEmpty()) item { Text("Дизлайкнутых треков нет.", color = C.faint, fontSize = 13.sp, modifier = Modifier.padding(horizontal = 20.dp)) }
            items(wave.dislikedList, key = { "d" + it.id }) { t ->
                Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(t.title, color = C.text, fontSize = 15.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(t.artist, color = C.muted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    Text("Вернуть", color = C.accent, fontSize = 13.sp, modifier = Modifier.pressable { wave.undislike(t.id) }.padding(8.dp))
                }
            }
        }
    }
}

@Composable
private fun WaveLabel(text: String) {
    Text(text, color = C.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(start = 20.dp, top = 16.dp, bottom = 6.dp))
}

// -------------------------------------------------------------- collection

/** A mix, a playlist or the uploads of an artist, over the tabs. */
@Composable
internal fun CollectionScreen(app: App, opened: Opened, shown: Shown, play: (List<Track>, Int) -> Unit, onClose: () -> Unit) {
    var tracks by remember(opened) { mutableStateOf<List<Track>?>(null) }
    var failed by remember(opened) { mutableStateOf<String?>(null) }
    var actionsFor by remember { mutableStateOf<Track?>(null) }
    actionsFor?.let { tr -> TrackActions(app, tr, false, fromPlaylist = opened.playlistId, onRemoved = { tracks = tracks?.filter { it.id != tr.id } }) { actionsFor = null } }
    LaunchedEffect(opened) {
        try {
            tracks = withContext(Dispatchers.IO) { opened.load() }
        } catch (e: Exception) {
            failed = "Не получилось загрузить: ${e.message}"
        }
    }
    Column(Modifier.fillMaxSize().background(C.bg).statusBarsPadding().navigationBarsPadding().consumeTaps()) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(44.dp).pressable(onClose), contentAlignment = Alignment.Center) { Icon(Ico.Back, 26.dp) }
            Text(opened.title, color = C.text, fontSize = 20.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
            // an artist's page: follow, through the computer (it holds the session that writes to SoundCloud)
            opened.artist?.let { a ->
                var following by remember(a.id) { mutableStateOf(false) }
                LaunchedEffect(a.id) { following = runCatching { withContext(Dispatchers.IO) { app.api.followings().any { it.id == a.id } } }.getOrDefault(false) }
                val scopeF = rememberCoroutineScope()
                val ctxF = androidx.compose.ui.platform.LocalContext.current
                Text(
                    if (following) "Вы подписаны" else "Подписаться", color = C.accent, fontSize = 14.sp, fontWeight = FontWeight.SemiBold,
                    modifier = Modifier.pressable {
                        val on = !following
                        following = on
                        scopeF.launch {
                            runCatching { Writes.call(app, "follow", org.json.JSONObject().put("id", a.id).put("name", a.name).put("avatar", a.avatar ?: "").put("on", on)) }
                                .onFailure { following = !on; android.widget.Toast.makeText(ctxF, L.t("Не получилось: {0}").replace("{0}", it.message ?: ""), android.widget.Toast.LENGTH_LONG).show() }
                        }
                    }.padding(12.dp),
                )
            }
        }
        failed?.let { Text(it, color = C.accent, fontSize = 14.sp, modifier = Modifier.padding(20.dp)) }
        val list = tracks
        if (list == null && failed == null) Text("Загружаем…", color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(20.dp))
        if (list != null) {
            val scopeC = rememberCoroutineScope()
            val ctxC = androidx.compose.ui.platform.LocalContext.current
            var likingAll by remember { mutableStateOf(false) }
            Row(Modifier.padding(horizontal = 16.dp, vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Box(Modifier.weight(1f)) { BigButton("Слушать", true) { play(list, 0) } }
                Box(Modifier.weight(1f)) { BigButton("Перемешать", false) { play(Independent.smartShuffle(list), 0) } }
            }
            // every track of the list into the likes (the computer does it, in the order that keeps the list's order)
            Text(
                if (likingAll) "Лайкаем…" else "Лайкнуть всё", color = C.accent, fontSize = 14.sp, fontWeight = FontWeight.SemiBold,
                modifier = Modifier.padding(horizontal = 20.dp, vertical = 4.dp).pressable {
                    if (likingAll) return@pressable
                    likingAll = true
                    scopeC.launch {
                        runCatching { Writes.likeMany(app, list.map { it.id }) }
                            .onSuccess { android.widget.Toast.makeText(ctxC, L.t("Готово"), android.widget.Toast.LENGTH_SHORT).show() }
                            .onFailure { android.widget.Toast.makeText(ctxC, L.t("Не получилось: {0}").replace("{0}", it.message ?: ""), android.widget.Toast.LENGTH_LONG).show() }
                        likingAll = false
                    }
                },
            )
            LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 96.dp)) {
                itemsIndexed(list, key = { _, t -> t.id }) { i, t -> TrackRow(t, shown.track?.id == t.id, onLong = { actionsFor = t }) { play(list, i) } }
            }
        }
    }
}

// -------------------------------------------------------------- categories

/** «Категории»: the same tiles as on the computer; a tile opens its tracks. */
@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
private fun CategoriesList(app: App, open: (Opened) -> Unit) {
    var years by remember { mutableStateOf(false) }
    if (years) androidx.compose.material3.AlertDialog(
        onDismissRequest = { years = false }, containerColor = C.panel,
        title = { Text("Хиты года", color = C.text) },
        text = { androidx.compose.foundation.layout.FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) { app.charts.yearList.forEach { y -> Chip(y.toString(), false) { years = false; open(Opened("Хиты " + y) { chartTracks(app, "ru:" + y, 50) }) } } } },
        confirmButton = { Text("Закрыть", color = C.muted, modifier = Modifier.pressable { years = false }.padding(12.dp)) },
    )
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 96.dp)) {
        Categories.groups.forEach { g ->
            item(key = "g-" + g.title) { Text(g.title.uppercase(), color = C.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(start = 20.dp, top = 16.dp, bottom = 6.dp)) }
            items(g.items.chunked(2), key = { row -> row[0].key }) { row ->
                Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    row.forEach { c ->
                        Column(
                            Modifier.weight(1f).clip(RoundedCornerShape(16.dp)).background(C.panel).border(1.dp, C.border, RoundedCornerShape(16.dp))
                                .pressable { if (c.key == "chart:years") years = true else open(Opened(c.title) { loadCategory(app, c) }) }.padding(14.dp),
                        ) {
                            // the covers of the first tracks, like the fan of records on the computer (the charts of the computer and its picks have none here)
                            val covers by produceState(listOf<String?>(), c.key) {
                                value = withContext(Dispatchers.IO) {
                                    runCatching { tileCovers(app, c) }.getOrDefault(emptyList())
                                }
                            }
                            // the mark of the tile, as on the computer
                            Text(when { c.kind == "pick" -> "только у нас"; c.kind == "chart" -> if (c.arg.matches(Regex(".*:[0-9]{4}"))) "по годам" else "№1 сейчас"; c.kind == "mix" -> "№1 сегодня"; else -> "лучшее" }, color = C.accent, fontSize = 10.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(bottom = 6.dp).clip(RoundedCornerShape(8.dp)).background(C.warm).padding(horizontal = 8.dp, vertical = 3.dp))
                            Box(Modifier.fillMaxWidth().height(66.dp).padding(bottom = 6.dp)) {
                                if (covers.isEmpty()) Box(Modifier.size(60.dp).clip(RoundedCornerShape(10.dp)).background(C.panel2), contentAlignment = Alignment.Center) { Text(c.title.take(1).uppercase(), color = C.accent, fontSize = 18.sp, fontWeight = FontWeight.Bold) }
                                else covers.take(3).withIndex().reversed().forEach { (i, u) -> Cover(u, 60.dp, 10.dp, Modifier.offset(x = (i * 32).dp)) }
                            }
                            Text(c.title, color = C.text, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, maxLines = 2, overflow = TextOverflow.Ellipsis)
                            Text(c.note.ifEmpty { when (c.kind) { "mix" -> "живой чарт SoundCloud"; "chart" -> "чарт Яндекс Музыки"; "pick" -> "подборка SC Desk"; else -> "по вашему поиску" } }, color = C.muted, fontSize = 12.sp, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(top = 4.dp))
                        }
                    }
                    if (row.size == 1) Spacer(Modifier.weight(1f))
                }
            }
        }
    }
}

/** The tracks of a tile (blocking, runs off the main thread). The charts are matched here; the picks are made of what this phone knows. */
internal fun loadCategory(app: App, c: Category): List<Track> = when (c.kind) {
    "mix" -> app.api.mixTracks(c.arg)
    "queries" -> app.api.byQueries(c.queries)
    "chart" -> chartTracks(app, c.arg, 50)
    else -> pickTracks(app, c.arg)
}

/** The chart "ru:day", "world:day", "ru:2019"; the sums over a week, a month and a year are kept by the computer, so only it knows them. */
internal fun chartTracks(app: App, kind: String, limit: Int): List<Track> {
    val (region, period) = kind.split(':').let { it[0] to it.getOrElse(1) { "day" } }
    val year = period.toIntOrNull()
    val songs = when {
        year != null -> app.charts.year(year)
        period == "day" -> app.charts.live(if (region == "world") "world" else "russia")
        else -> {
            // week, month, year: the computer sums its daily records
            val data = kotlinx.coroutines.runBlocking { app.link.rpc("chart", org.json.JSONObject().put("kind", kind)) }.getJSONArray("data")
            return runCatching { app.api.tracksByIds((0 until data.length()).map { data.getJSONObject(it).optLong("id") }) }.getOrDefault(emptyList())
        }
    }
    return kotlinx.coroutines.runBlocking { app.charts.tracks(songs, limit) }
}

/** «Только в SC Desk»: the picks, made here of the likes and the plays of this phone. */
private fun pickTracks(app: App, kind: String): List<Track> {
    val day = 86_400_000L
    val now = System.currentTimeMillis()
    val plays = History.all(app.store)
    return when (kind) {
        // liked long ago and not played here lately
        "forgotten" -> {
            val heard = plays.filter { it.at > now - 60 * day }.map { it.id }.toSet()
            app.api.likesDated(200).filter { it.second in 1..(now - 90 * day) && it.first.id !in heard }.map { it.first }.shuffled().take(50)
        }
        // what was liked about this time a year ago
        "year-ago" -> app.api.likesDated(300).filter { Math.abs(it.second - (now - 365 * day)) < 45 * day }.sortedBy { it.second }.map { it.first }.take(50)
        // the most played of the month
        "repeat" -> plays.filter { it.at > now - 30 * day }.groupBy { it.id }.values.sortedByDescending { it.size }.take(50).map { g -> g[0].let { Track(it.id, it.title, it.artist, it.durationMs, it.artwork) } }
        // little known artists in the taste: what is related to the likes and has few plays
        "radar" -> {
            val likes = app.api.likesDated(60).map { it.first }
            val known = likes.map { it.id }.toSet()
            val out = LinkedHashMap<Long, Track>()
            for (l in likes.shuffled().take(6)) runCatching { app.api.related(l.id) }.getOrDefault(emptyList()).filter { it.id !in known && app.api.plays(it.id) in 1..20_000 }.forEach { out.putIfAbsent(it.id, it) }
            out.values.take(50)
        }
        // noise and distortion
        "anti" -> app.api.byQueries(listOf("harsh noise", "noise experimental", "антимузыка"))
        else -> emptyList()
    }
}

/** The covers of the first tracks of a tile (blocking). */
internal fun tileCovers(app: App, c: Category): List<String?> = app.fans.getOrPut(c.key) {
    when (c.kind) {
        "mix" -> app.api.mixCovers(c.arg)
        "queries" -> app.api.search(c.queries.firstOrNull() ?: "").take(3).map { it.artwork }
        "chart" -> chartTracks(app, c.arg, 4).take(3).map { it.artwork }
        // the picks have no list of their own to show here: the newest likes stand for them
        else -> if (app.store.signedIn) runCatching { app.api.likes(null, 3).first.map { it.artwork } }.getOrDefault(emptyList()) else emptyList()
    }
}
