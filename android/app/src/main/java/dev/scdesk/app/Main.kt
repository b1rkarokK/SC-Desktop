package dev.scdesk.app

import android.os.SystemClock
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
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
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import org.json.JSONArray
import org.json.JSONObject
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private enum class Tab { Home, Search, Wave, Likes, More }

/** The app: the lists of SoundCloud with the player over them; the account and the computer live in «Ещё». */
@Composable
fun Root(app: App, pairLink: String? = null) {
    val scope = rememberCoroutineScope()
    val state by app.engine.state.collectAsState()
    val peers by app.link.peers.collectAsState()
    val now by produceState(SystemClock.elapsedRealtime()) { while (true) { delay(3000); value = SystemClock.elapsedRealtime() } }
    val room by app.rooms.state.collectAsState()
    val following by app.rooms.following.collectAsState()
    // the room is looked at now and then (the computer may be in one), and when the phone listens to it it plays the same
    LaunchedEffect(Unit) { while (true) { if (app.store.linked && !app.rooms.following.value) app.rooms.refresh(); delay(20_000) } }
    // which device the player on the screen shows and drives: null, the one that plays (the phone first); "", this phone; else the id of another device.
    // The phone and the computer play each their own track at the same time: nothing here stops the other one
    var control by remember { mutableStateOf<String?>(null) }
    val picked = when (control) { null -> activeRemote(peers, state.playing, now); "" -> null; else -> peers[control] }
    // the other device's player does not vanish for a moment when one of its reports is late or has no track (a skip over there)
    var lastRemote by remember { mutableStateOf<Peer?>(null) }
    var lastRemoteAt by remember { mutableStateOf(0L) }
    if (picked != null) { lastRemote = picked; lastRemoteAt = now }
    LaunchedEffect(following, peers) {
        if (!following) return@LaunchedEffect
        val pc = peers.values.firstOrNull { it.kind == "pc" } ?: return@LaunchedEffect
        val tr = pc.track ?: return@LaunchedEffect
        if (!pc.playing) { app.engine.pause(); return@LaunchedEffect }
        // where the computer's player is now (it tells every few seconds; the rest is counted)
        val there = pc.posMs + (SystemClock.elapsedRealtime() - pc.posAt) + 250
        val here = app.engine.state.value.track
        if (here?.id != tr.id) {
            val l = withContext(Dispatchers.IO) { runCatching { app.api.tracksByIds(listOf(tr.id)) }.getOrNull() }
            if (!l.isNullOrEmpty()) app.engine.playQueue(l, 0, positionMs = there)
        } else {
            if (kotlin.math.abs(app.engine.position() - there) > 1_500) app.engine.seek(there)
            if (!app.engine.state.value.playing) app.engine.play()
        }
    }
    val remote = picked ?: lastRemote?.takeIf { control == null && !state.playing && now - lastRemoteAt < 15_000 && now - it.seenAt < 60_000 }
    val shown = if (remote != null) Shown(remote.track, true, remote, false) else Shown(state.track, state.playing, null, state.loading)

    var tab by remember { mutableStateOf(Tab.Home) }
    var opened by remember { mutableStateOf<Opened?>(null) }
    // the wave: the tracks it queued (what comes after the playing one is shown), the mood, and a batch being made
    val waveList = remember { mutableStateListOf<Track>() }
    var vibe by remember { mutableStateOf("any") }
    var waveBusy by remember { mutableStateOf(false) }
    var big by remember { mutableStateOf(false) }
    var devices by remember { mutableStateOf(false) }
    var scan by remember { mutableStateOf(pairLink != null) }
    var eqOpen by remember { mutableStateOf(false) }
    /** a page over the tabs: "settings", "history" or "stats" */
    var page by remember { mutableStateOf<String?>(null) }
    var lyricsOpen by remember { mutableStateOf(false) }
    var update by remember { mutableStateOf<UpdateInfo?>(null) }
    var updateMsg by remember { mutableStateOf<String?>(null) }
    val ctx = androidx.compose.ui.platform.LocalContext.current
    // a look at the feed of updates when the app opens (not more often than once an hour), and by hand in «Ещё»
    val checkUpdate: (Boolean) -> Unit = { manual ->
        scope.launch {
            try {
                val found = withContext(Dispatchers.IO) { Updater.check(BuildConfig.VERSION_CODE) }
                app.store.updateCheckedAt = System.currentTimeMillis()
                update = found
                if (manual) updateMsg = if (found == null) "У вас последняя версия" else null
            } catch (e: Exception) {
                if (manual) updateMsg = "Не удалось проверить: ${e.message}"
            }
        }
    }
    LaunchedEffect(Unit) {
        if (app.store.autoUpdate && System.currentTimeMillis() - app.store.updateCheckedAt > 3600 * 1000L) checkUpdate(false)
    }
    var login by remember { mutableStateOf(false) }
    /** bumped when the account changes: the lists load again */
    var account by remember { mutableIntStateOf(0) }
    val signOut: () -> Unit = {
        app.link.stop()
        app.engine.pause()
        app.store.signOut()
        android.webkit.CookieManager.getInstance().removeAllCookies(null)
        account++
    }
    // the accent from the cover of the playing track, when it is asked for
    LaunchedEffect(shown.track?.artwork, C.useCover) {
        if (!C.useCover) { C.coverAccent = null; return@LaunchedEffect }
        val art = shown.track?.artwork ?: return@LaunchedEffect
        // the cover is on the screen a moment after the track is
        delay(700)
        C.coverAccent = withContext(Dispatchers.IO) { CoverCache.accentOf(art) }
    }
    LaunchedEffect(pairLink) { if (pairLink != null) scan = true }

    // a track picked here plays here: what plays on the computer is stopped
    val play: (List<Track>, Int) -> Unit = { list, i ->
        control = null
        app.engine.playQueue(list.drop(i).take(100), 0)
    }

    // starts or renews the wave: a batch from the likes and what SoundCloud finds related to them
    val startWave: () -> Unit = {
        if (!waveBusy) {
            waveBusy = true
            app.wave.vibe = vibe
            scope.launch {
                try {
                    val batch = withContext(Dispatchers.IO) { app.wave.batch() }
                    if (batch.isNotEmpty()) {
                        control = null
                        waveList.clear()
                        waveList.addAll(batch)
                        app.engine.playQueue(batch, 0, wave = true)
                    }
                } catch (e: Exception) {
                    android.util.Log.w("SCDesk", "wave: ${e.message}")
                }
                waveBusy = false
            }
        }
    }
    // the wave goes on: when few tracks are left, the next batch is added
    LaunchedEffect(state.track?.id, app.wave.active) {
        if (!app.wave.active || waveBusy) return@LaunchedEffect
        app.engine.remaining { left ->
            if (left <= 5 && !waveBusy) {
                waveBusy = true
                scope.launch {
                    try {
                        val more = withContext(Dispatchers.IO) { app.wave.batch() }
                        if (app.wave.active && more.isNotEmpty()) {
                            waveList.addAll(more)
                            app.engine.append(more)
                        }
                    } catch (e: Exception) {
                        android.util.Log.w("SCDesk", "wave more: ${e.message}")
                    }
                    waveBusy = false
                }
            }
        }
    }

    // the system Back closes the topmost layer instead of leaving the app
    androidx.activity.compose.BackHandler(enabled = lyricsOpen || eqOpen || page != null || devices || big || opened != null || scan || login || tab != Tab.Home) {
        when {
            lyricsOpen -> lyricsOpen = false
            eqOpen -> eqOpen = false
            scan -> scan = false
            login -> login = false
            devices -> devices = false
            page != null -> page = null
            big -> big = false
            opened != null -> opened = null
            else -> tab = Tab.Home
        }
    }

    Box(Modifier.fillMaxSize().background(C.bg)) {
        Column(Modifier.fillMaxSize().statusBarsPadding()) {
            room?.let { r ->
                Row(Modifier.fillMaxWidth().background(C.warm).padding(horizontal = 16.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
                    SIcon("rooms", C.accent)
                    Text("В комнате «${r.name}» · ${r.members.size}", color = C.accent, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(start = 10.dp).weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                    if (!following) Text("Слушать здесь", color = C.accent, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.pressable { control = ""; app.rooms.follow() }.padding(8.dp))
                    Text("Выйти", color = C.text, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.pressable { app.rooms.stop(leave = true) }.padding(8.dp))
                }
            }
            Box(Modifier.weight(1f)) {
                when (tab) {
                    Tab.Home -> HomeTab(app, shown, account, play, open = { opened = it }, onLogin = { login = true }, onPage = { page = it })
                    Tab.Search -> SearchTab(app, shown, play, open = { opened = it })
                    Tab.Wave -> WaveTab(app, shown, waveList, vibe, waveBusy, onVibe = { vibe = it; app.wave.vibe = it; if (app.wave.active) startWave() }, onStart = startWave, onLogin = { login = true }, play = play, onDislike = { t -> app.wave.dislike(t); waveList.removeAll { it.id == t.id }; app.engine.removeId(t.id); if (t.id == state.track?.id) app.engine.next() })
                    Tab.Likes -> LikesTab(app, shown, account, play, open = { opened = it }, onLogin = { login = true }, onDevices = { devices = true }, remote = remote != null)
                    Tab.More -> MoreTab(
                        app, account,
                        onLogin = { login = true },
                        onScan = { scan = true },
                        onPage = { page = it },
                        onCheckUpdate = { updateMsg = "Проверяем…"; checkUpdate(true) },
                        updateMsg = updateMsg,
                        onSignOut = signOut,
                    )
                }
            }
            // the mini player sits over the navigation
            shown.track?.let { t ->
                val progress by produceState(0f, t.id, shown.playing, remote?.dev) {
                    while (true) {
                        val dur = if (remote != null) t.durationMs else state.durationMs.takeIf { it > 0 } ?: t.durationMs
                        val p = if (remote != null) remote.posMs + if (remote.playing) SystemClock.elapsedRealtime() - remote.posAt else 0 else app.engine.position()
                        value = if (dur > 0) p.toFloat() / dur else 0f
                        delay(1000)
                    }
                }
                MiniPlayer(t, shown, Modifier.padding(horizontal = 8.dp, vertical = 6.dp), progress = { progress }, roomLine = if (following) room?.let { r -> listOfNotNull(r.name, r.who(r.addedBy)?.let { "добавил(а) $it" }).joinToString(" · ") } else null, onOpen = { big = true }) {
                    if (remote != null) app.link.command(remote.dev, if (shown.playing) "pause" else "play") else app.engine.toggle()
                }
            }
            NavBar(tab) { tab = it }
        }

        opened?.let { o -> CollectionScreen(app, o, shown, play, onClose = { opened = null }) }

        AnimatedVisibility(
            big && shown.track != null,
            enter = slideInVertically(spring(dampingRatio = 0.85f, stiffness = Spring.StiffnessMediumLow)) { it } + fadeIn(),
            exit = slideOutVertically { it } + fadeOut(),
        ) {
            shown.track?.let { PlayerScreen(app, shown, it, onClose = { big = false }, onDevices = { devices = true }, onLyrics = { lyricsOpen = true }, onEq = { eqOpen = true }, onRooms = { big = false; page = "rooms" }) }
        }

        AnimatedVisibility(
            devices,
            enter = slideInVertically(spring(dampingRatio = 0.9f, stiffness = Spring.StiffnessMediumLow)) { it } + fadeIn(),
            exit = slideOutVertically { it } + fadeOut(),
        ) {
            DevicesPanel(app, shown, onClose = { devices = false }, onLinkPc = { devices = false; scan = true }, onEq = { devices = false; eqOpen = true }, onControl = { control = it })
        }

        update?.let { u -> UpdateDialog(u, ctx, onLater = { update = null }) }
        when (page) {
            "history" -> HistoryScreen(app, shown, play) { page = null }
            "stats" -> StatsScreen(app) { page = null }
            "downloads" -> DownloadsScreen(app, shown, play) { page = null }
            "friends" -> FriendsScreen(app, shown, play) { page = null }
            "playlists" -> SharedScreen(app, shown, play) { page = null }
            "rooms" -> RoomsScreen(app) { page = null }
            "settings" -> SettingsScreen(app, onEq = { eqOpen = true }, onUnlinked = { account++ }, onPage = { page = it }, onSignOut = signOut, onCheckUpdate = { updateMsg = "Проверяем…"; checkUpdate(true) }, updateMsg = updateMsg) { page = null }
        }
        if (lyricsOpen) shown.track?.let { LyricsScreen(app, shown, it) { lyricsOpen = false } }
        if (eqOpen) EqScreen(app) { eqOpen = false }
        if (scan) ScanScreen(app, pairLink, onBack = { scan = false }, onLogin = { scan = false; login = true }) {
            scan = false
            account++
            app.engine.connect()
            app.link.refreshConnection()
        }
        if (login) LoginScreen(app, onBack = { login = false }) {
            login = false
            account++
            app.engine.connect()
        }
    }
}

@Composable
private fun NavBar(tab: Tab, onPick: (Tab) -> Unit) {
    Row(Modifier.fillMaxWidth().background(C.bg).navigationBarsPadding().padding(top = 4.dp), horizontalArrangement = Arrangement.SpaceEvenly) {
        listOf(Triple(Tab.Home, Ico.Home, "Главная"), Triple(Tab.Search, Ico.Search, "Поиск"), Triple(Tab.Wave, Ico.Wave, "Волна"), Triple(Tab.Likes, Ico.Heart, "Лайки"), Triple(Tab.More, Ico.Menu, "Ещё")).forEach { (t, icon, label) ->
            val on = tab == t
            Column(Modifier.weight(1f).pressable { onPick(t) }.padding(vertical = 8.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Icon(if (t == Tab.Likes && !on) Ico.HeartOff else icon, 26.dp, if (on) C.accent else C.muted)
                Text(label, color = if (on) C.accent else C.muted, fontSize = 11.sp, fontWeight = if (on) FontWeight.SemiBold else FontWeight.Normal)
            }
        }
    }
}

@Composable
internal fun Head(title: String, trailing: @Composable () -> Unit = {}) {
    Row(Modifier.fillMaxWidth().padding(start = 20.dp, end = 12.dp, top = 12.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(title, color = C.text, fontSize = 26.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
        trailing()
    }
}

@Composable
internal fun TrackRow(t: Track, current: Boolean, trailing: (@Composable () -> Unit)? = null, onClick: () -> Unit) {
    val pad = when (C.density) { 0 -> 3.dp; 2 -> 11.dp; else -> 7.dp }
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 8.dp).clip(RoundedCornerShape(14.dp)).background(if (current) C.panel else androidx.compose.ui.graphics.Color.Transparent).pressable(onClick).padding(horizontal = 12.dp, vertical = pad),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Cover(t.artwork, if (C.density == 0) 42.dp else 48.dp, 8.dp)
        Column(Modifier.padding(start = 14.dp).weight(1f)) {
            Text(t.title, color = if (current) C.accent else C.text, fontSize = 16.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(t.artist, color = C.muted, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        trailing?.invoke()
    }
}

// ------------------------------------------------------------------ search

@Composable
private fun SearchTab(app: App, shown: Shown, play: (List<Track>, Int) -> Unit, open: (Opened) -> Unit) {
    var query by remember { mutableStateOf("") }
    // 0 all, 1 tracks, 2 artists, 3 playlists, 4 by the words of a song
    var kind by remember { mutableStateOf(0) }
    var tracks by remember { mutableStateOf(listOf<Track>()) }
    var artists by remember { mutableStateOf(listOf<Artist>()) }
    var playlists by remember { mutableStateOf(listOf<Card>()) }
    var lines by remember { mutableStateOf(mapOf<Long, String>()) }
    var error by remember { mutableStateOf<String?>(null) }
    var recents by remember { mutableStateOf(app.store.recents) }
    val byLyrics = kind == 4
    // a little after the last letter, not on every one
    LaunchedEffect(query, kind) {
        if (query.trim().length < (if (byLyrics) 4 else 2)) { tracks = emptyList(); artists = emptyList(); playlists = emptyList(); return@LaunchedEffect }
        delay(if (byLyrics) 900 else 450)
        try {
            val q = query.trim()
            if (byLyrics) {
                val arr = app.link.rpc("lsearch", org.json.JSONObject().put("q", q)).getJSONArray("data")
                val objs = (0 until arr.length()).map { arr.getJSONObject(it) }
                tracks = objs.map { trackOf(it) }
                lines = objs.filter { it.optString("line").isNotEmpty() }.associate { it.optLong("id") to it.optString("line") }
            } else withContext(Dispatchers.IO) {
                if (kind == 0 || kind == 1) tracks = app.api.search(q)
                if (kind == 0 || kind == 2) artists = runCatching { app.api.searchUsers(q) }.getOrDefault(emptyList())
                if (kind == 0 || kind == 3) playlists = runCatching { app.api.searchPlaylists(q) }.getOrDefault(emptyList())
            }
            error = null
            // what was looked for is remembered (the newest first, eight at most)
            if (q.length >= 3) { recents = (listOf(q) + recents.filter { !it.equals(q, true) }).take(8); app.store.recents = recents }
        } catch (e: Exception) {
            error = if (byLyrics) "Поиск по тексту работает через компьютер: ${e.message}. Включите SC Desk на нём." else "Поиск не отвечает: ${e.message}"
        }
    }
    Column(Modifier.fillMaxSize()) {
        Spacer(Modifier.height(10.dp))
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).clip(RoundedCornerShape(16.dp)).background(C.panel).border(1.dp, C.border, RoundedCornerShape(16.dp)).padding(horizontal = 14.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            SIcon("search", C.muted)
            Box(Modifier.padding(start = 10.dp).weight(1f)) {
                if (query.isEmpty()) Text(if (byLyrics) "Слова из песни" else "Треки, артисты, плейлисты", color = C.faint, fontSize = 16.sp)
                BasicTextField(query, { query = it }, singleLine = true, textStyle = TextStyle(color = C.text, fontSize = 16.sp), cursorBrush = SolidColor(C.accent), modifier = Modifier.fillMaxWidth())
            }
            if (query.isNotEmpty()) Box(Modifier.size(28.dp).pressable { query = "" }, contentAlignment = Alignment.Center) { SIcon("close", C.muted) }
        }
        Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 2.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf("Всё", "Треки", "Артисты", "Плейлисты", "По тексту").forEachIndexed { i, l -> Chip(l, kind == i) { kind = i } }
        }
        error?.let { Text(it, color = C.accent, fontSize = 14.sp, modifier = Modifier.padding(20.dp)) }
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 96.dp)) {
            if (query.trim().length < 2 && recents.isNotEmpty()) {
                item { SectionHead("Недавнее") }
                items(recents.size, key = { "r" + recents[it] }) { k ->
                    Row(Modifier.fillMaxWidth().pressable { query = recents[k] }.padding(horizontal = 20.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
                        SIcon("clock", C.muted)
                        Text(recents[k], color = C.text, fontSize = 16.sp, fontWeight = FontWeight.Medium, modifier = Modifier.padding(start = 14.dp).weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Box(Modifier.size(32.dp).pressable { recents = recents - recents[k]; app.store.recents = recents }, contentAlignment = Alignment.Center) { SIcon("close", C.faint) }
                    }
                }
            }
            val tr = if (kind == 0) tracks.take(5) else tracks
            if (tr.isNotEmpty() && kind != 2 && kind != 3) {
                if (kind == 0) item { SectionHead("Треки") }
                itemsIndexed(tr, key = { _, t -> "t" + t.id }) { i, t ->
                    Column {
                        TrackRow(t, shown.track?.id == t.id) { play(tracks, i) }
                        if (byLyrics) lines[t.id]?.let { Text("«$it»", color = C.accent, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 82.dp, end = 20.dp)) }
                    }
                }
            }
            val ar = if (kind == 0) artists.take(3) else artists
            if (ar.isNotEmpty() && (kind == 0 || kind == 2)) {
                if (kind == 0) item { SectionHead("Артисты") }
                items(ar.size, key = { "a" + ar[it].id }) { k ->
                    val a = ar[k]
                    Row(Modifier.fillMaxWidth().pressable { open(Opened(a.name) { app.api.artistTracks(a.id) }) }.padding(horizontal = 20.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                        Cover(a.avatar, 48.dp, 24.dp)
                        Text(a.name, color = C.text, fontSize = 16.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 14.dp))
                    }
                }
            }
            val pl = if (kind == 0) playlists.take(3) else playlists
            if (pl.isNotEmpty() && (kind == 0 || kind == 3)) {
                if (kind == 0) item { SectionHead("Плейлисты") }
                items(pl.size, key = { "p" + pl[it].id }) { k ->
                    val c = pl[k]
                    Row(Modifier.fillMaxWidth().pressable { open(Opened(c.title) { app.api.playlistTracks(c.id) }) }.padding(horizontal = 20.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                        Cover(c.artwork, 56.dp, 10.dp)
                        Column(Modifier.padding(start = 14.dp).weight(1f)) {
                            Text(c.title, color = C.text, fontSize = 16.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text(c.subtitle, color = C.muted, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun SectionHead(text: String) {
    Text(text, color = C.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(start = 20.dp, top = 16.dp, bottom = 4.dp))
}

// -------------------------------------------------------------------- more

@Composable
internal fun BigButton(text: String, accent: Boolean, onClick: () -> Unit) {
    Text(
        text, color = C.bg, fontSize = 16.sp, fontWeight = FontWeight.SemiBold, textAlign = TextAlign.Center,
        modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(if (accent) C.accent else C.text).pressable(onClick).padding(vertical = 16.dp),
    )
}

@Composable
private fun Card(content: @Composable () -> Unit) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp).clip(RoundedCornerShape(18.dp)).background(C.panel).border(1.dp, C.border, RoundedCornerShape(18.dp)).padding(16.dp)) { content() }
}

@Composable
private fun MoreTab(app: App, account: Int, onLogin: () -> Unit, onScan: () -> Unit, onPage: (String) -> Unit, onCheckUpdate: () -> Unit, updateMsg: String?, onSignOut: () -> Unit) {
    // `account` makes this read again when the account changes
    val signedIn = remember(account) { app.store.signedIn }
    val linked = remember(account) { app.store.linked }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
        Head("Ещё")
        if (signedIn) {
            // the account: a card that opens the settings of the account
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp).clip(RoundedCornerShape(18.dp)).background(C.warm).border(1.dp, C.border, RoundedCornerShape(18.dp)).pressable { onPage("settings") }.padding(14.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                val name = app.store.userName.ifEmpty { "Аккаунт" }
                Box(Modifier.size(46.dp).clip(androidx.compose.foundation.shape.CircleShape).background(C.panel2), contentAlignment = Alignment.Center) {
                    Text(name.take(1).uppercase(), color = C.accent, fontSize = 18.sp, fontWeight = FontWeight.Bold)
                }
                Column(Modifier.padding(start = 14.dp).weight(1f)) {
                    Text(name, color = C.text, fontSize = 17.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text("аккаунт SoundCloud", color = C.muted, fontSize = 12.sp)
                }
                SIcon("chev", C.muted)
            }
        } else {
            Card {
                Text("Вы не вошли", color = C.text, fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
                Text("Поиск и музыка работают и так. Лайки и подборки появятся после входа.", color = C.muted, fontSize = 13.sp, modifier = Modifier.padding(vertical = 8.dp))
                BigButton("Войти через SoundCloud", true, onLogin)
                Spacer(Modifier.height(8.dp))
                BigButton("Войти с помощью компьютера", false, onScan)
                Text("Если на компьютере в SC Desk уже есть вход, наведите камеру на его QR-код: пароль не нужен. Вход через Google или Apple напрямую здесь не работает, для него это единственный путь.", color = C.faint, fontSize = 12.sp, modifier = Modifier.padding(top = 8.dp))
            }
        }
        if (!linked) Card {
            Text("Компьютер", color = C.text, fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
            Text("Наведите камеру на QR-код из SC Desk: музыку можно будет играть то на компьютере, то на телефоне. Если на компьютере нет входа, он возьмёт его с телефона.", color = C.muted, fontSize = 13.sp, modifier = Modifier.padding(vertical = 8.dp))
            BigButton("Подключить компьютер", false, onScan)
            Text("На компьютере: Настройки, Интеграции, «Подключить телефон» или «Войти по QR с телефона».", color = C.faint, fontSize = 12.sp, modifier = Modifier.padding(top = 8.dp))
        }
        Column(Modifier.padding(horizontal = 16.dp, vertical = 6.dp)) {
            Row1("rooms", "Друзья", "") { onPage("friends") }
            Row1("queue", "Общие плейлисты", "") { onPage("playlists") }
            Row1("radio", "Комнаты", "") { onPage("rooms") }
            Row1("history", "История", "") { onPage("history") }
            Row1("chart", "Итоги", "") { onPage("stats") }
            Row1("download", "Скачанное", "") { onPage("downloads") }
            Row1("gear", "Настройки", "") { onPage("settings") }
        }
        Spacer(Modifier.height(96.dp))
    }
}

/** The way out to the network: the usual one, or a server of the VPN subscription (given by the computer; it works where the provider lets only allowed sites in). */
@Composable
internal fun NetworkCard(app: App) {
    val servers by app.vpnServers.collectAsState()
    val waiting by app.vpnWaiting.collectAsState()
    val note by app.vpnNote.collectAsState()
    var sel by remember { mutableIntStateOf(app.store.vpnSelected) }
    var status by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val ctx = androidx.compose.ui.platform.LocalContext.current
    fun pick(i: Int) {
        if (busy) return
        busy = true
        status = if (i < 0) "Обычное соединение…" else "Подключаемся…"
        scope.launch {
            val msg = withContext(Dispatchers.IO) {
                if (i < 0) {
                    Tunnel.stop(ctx); Tunnel.selected = -1
                    "Обычное соединение"
                } else if (!Tunnel.available(ctx)) {
                    "В этой сборке нет движка серверов"
                } else if (!Tunnel.start(ctx, servers[i])) {
                    "Сервер не запустился"
                } else {
                    val ms = Tunnel.test()
                    if (ms < 0) { Tunnel.stop(ctx); "Сайт через этот сервер не открылся, выберите другой" } else { Tunnel.selected = i; "Работает, ответ ${ms} мс" }
                }
            }
            val ok = (i < 0) || Tunnel.port != 0
            if (ok) { sel = i; app.store.vpnSelected = i }
            status = msg
            busy = false
        }
    }
    Card {
        Text("Сеть", color = C.text, fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
        Text(
            if (sel >= 0 && sel < servers.size) "Через сервер: " + servers[sel].name else "Обычное соединение. Если SoundCloud не открывается или есть белые списки, выберите сервер из своей VPN-подписки.",
            color = C.muted, fontSize = 13.sp, modifier = Modifier.padding(vertical = 8.dp),
        )
        if (servers.isNotEmpty()) {
            Column(Modifier.heightIn(max = 260.dp).verticalScroll(rememberScrollState())) {
                Text("Обычное соединение", color = if (sel < 0) C.accent else C.text, fontSize = 15.sp, modifier = Modifier.fillMaxWidth().pressable { pick(-1) }.padding(vertical = 10.dp))
                servers.forEachIndexed { i, sv ->
                    Text(sv.name.ifEmpty { "Сервер ${i + 1}" }, color = if (sel == i) C.accent else C.text, fontSize = 15.sp, maxLines = 1, modifier = Modifier.fillMaxWidth().pressable { pick(i) }.padding(vertical = 10.dp))
                }
            }
            Spacer(Modifier.height(8.dp))
        }
        (status ?: note)?.let { Text(it, color = C.faint, fontSize = 12.sp, modifier = Modifier.padding(bottom = 8.dp)) }
        BigButton(if (waiting) "Ждём компьютер…" else "Получить серверы с компьютера", false) { if (!waiting && app.store.linked) app.link.requestVpn() else if (!app.store.linked) app.vpnNote.value = "Сначала подключите компьютер." }
        Text("Компьютер спросит разрешение. Подписка не считает телефон ещё одним устройством: серверы берутся с компьютера.", color = C.faint, fontSize = 12.sp, modifier = Modifier.padding(top = 8.dp))
        // without a computer: a subscription address or one server link, pasted here
        var added by remember { mutableStateOf("") }
        var adding by remember { mutableStateOf(false) }
        Spacer(Modifier.height(14.dp))
        Text("Добавить без компьютера", color = C.text, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
        Box(Modifier.padding(top = 8.dp).fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(C.panel2).padding(horizontal = 14.dp, vertical = 12.dp)) {
            if (added.isEmpty()) Text("Ссылка на подписку или vless://…", color = C.faint, fontSize = 14.sp)
            BasicTextField(added, { added = it }, singleLine = true, textStyle = TextStyle(color = C.text, fontSize = 14.sp), cursorBrush = SolidColor(C.accent), modifier = Modifier.fillMaxWidth())
        }
        Spacer(Modifier.height(8.dp))
        BigButton(if (adding) "Читаем…" else "Добавить серверы", false) {
            if (adding || added.isBlank()) return@BigButton
            adding = true
            scope.launch {
                val r = withContext(Dispatchers.IO) { runCatching { VpnLinks.fetch(added) } }
                r.onSuccess { list ->
                    // what was here stays; a server with the same name is replaced
                    val old = app.vpnServers.value.filter { o -> list.none { it.name == o.name } }
                    val all = old + list
                    app.store.vpnServers = JSONArray().also { a -> all.forEach { sv -> a.put(JSONObject().put("n", sv.name).put("o", sv.outbound)) } }.toString()
                    app.vpnServers.value = all
                    app.vpnNote.value = "Добавлено серверов: ${list.size}"
                    added = ""
                }.onFailure { app.vpnNote.value = "Не вышло: ${it.message}" }
                adding = false
            }
        }
    }
}
