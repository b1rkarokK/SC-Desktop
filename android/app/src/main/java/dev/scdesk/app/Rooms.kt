package dev.scdesk.app

import androidx.compose.foundation.background
import androidx.compose.foundation.border
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
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.AlertDialog
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

/** A person in the room. */
data class RoomMember(val id: String, val name: String, val avatar: String?, val you: Boolean)

/** The room the computer is in, as the computer tells it (the rooms are lived through the computer; the phone listens to them and steers). */
data class RoomState(
    val channel: String, val name: String, val host: Boolean, val members: List<RoomMember>,
    val title: String?, val artist: String?, val addedBy: String?, val nextTitle: String?, val queue: Int,
    val voteSkip: Boolean, val skipVotes: Int, val skipNeed: Int,
) {
    fun who(id: String?): String? = members.firstOrNull { it.id == id }?.name
    /** «добавил(а) Аня · дальше «…»» */
    val line: String get() = listOfNotNull(who(addedBy)?.let { "добавил(а) $it" }, nextTitle?.let { "дальше «$it»" }).joinToString(" · ")

    companion object {
        fun parse(d: JSONObject): RoomState? {
            if (!d.optBoolean("active") || d.optString("channel").isEmpty()) return null
            val ms = d.optJSONArray("members")
            val members = if (ms == null) emptyList() else (0 until ms.length()).map { ms.getJSONObject(it) }.map { RoomMember(it.optString("id"), it.optString("name"), it.optString("avatar").takeIf { a -> a.isNotEmpty() && a != "null" }, it.optBoolean("you")) }
            val cur = d.optJSONObject("current")
            return RoomState(
                d.optString("channel"), d.optString("name"), d.optBoolean("host"), members,
                cur?.optString("title"), cur?.optString("artist"), cur?.optString("by"), d.optJSONObject("next")?.optString("title"), d.optInt("queue"),
                d.optBoolean("vote_skip"), d.optInt("skip_votes"), d.optInt("skip_need"),
            )
        }
    }
}

/** What the phone does about rooms: listens to the one the computer is in (the computer goes silent meanwhile) and keeps its state fresh. */
class Rooms(private val app: App) {
    val state = kotlinx.coroutines.flow.MutableStateFlow<RoomState?>(null)
    val following = kotlinx.coroutines.flow.MutableStateFlow(false)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var poll: kotlinx.coroutines.Job? = null

    /** Asks for the room now. */
    suspend fun refresh() {
        state.value = runCatching { RoomState.parse(app.link.rpc("room_view", waitMs = 1_500).getJSONObject("data")) }.getOrElse { state.value }
    }

    /** The phone plays what the room plays; the computer is silent while it does. */
    fun follow() {
        if (following.value) return
        following.value = true
        scope.launch { runCatching { app.link.rpc("mute", JSONObject().put("on", true)) } }
        poll?.cancel()
        poll = scope.launch {
            while (following.value) {
                refresh()
                if (state.value == null) { stop(); break }
                delay(3_000)
            }
        }
    }

    /** The phone stops listening, the computer gets its sound back. */
    fun stop(leave: Boolean = false) {
        val was = following.value
        following.value = false
        poll?.cancel()
        scope.launch {
            if (was) runCatching { app.link.rpc("mute", JSONObject().put("on", false)) }
            if (leave) runCatching { app.link.rpc("room_leave") }
            if (was) app.engine.pause()
            refresh()
        }
    }

    /** One look at the room (when the screens open and the phone is not following). */
    fun peek() { scope.launch { refresh() } }
}

@Composable
private fun Field(value: String, hint: String, onChange: (String) -> Unit, secret: Boolean = false) {
    Box(Modifier.fillMaxWidth().padding(vertical = 4.dp).clip(RoundedCornerShape(12.dp)).background(C.panel2).padding(horizontal = 14.dp, vertical = 12.dp)) {
        if (value.isEmpty()) Text(hint, color = C.faint, fontSize = 15.sp)
        BasicTextField(
            value, onChange, singleLine = true, textStyle = TextStyle(color = C.text, fontSize = 15.sp), cursorBrush = SolidColor(C.accent), modifier = Modifier.fillMaxWidth(),
            visualTransformation = if (secret) androidx.compose.ui.text.input.PasswordVisualTransformation() else androidx.compose.ui.text.input.VisualTransformation.None,
        )
    }
}

/** «Комнаты»: the rooms of this account, the live rooms of the friends, the way in by a link, and the new room. */
@Composable
internal fun RoomsScreen(app: App, onBack: () -> Unit) {
    val scope = rememberCoroutineScope()
    var tick by remember { mutableIntStateOf(0) }
    var mine by remember { mutableStateOf<List<JSONObject>?>(null) }
    var live by remember { mutableStateOf<List<JSONObject>>(emptyList()) }
    var failed by remember { mutableStateOf<String?>(null) }
    var create by remember { mutableStateOf(false) }
    var byLink by remember { mutableStateOf(false) }
    var ask by remember { mutableStateOf<JSONObject?>(null) }
    var note by remember { mutableStateOf<String?>(null) }
    val following by app.rooms.following.collectAsState()
    val inRoom by app.rooms.state.collectAsState()

    LaunchedEffect(tick) {
        try {
            val d = app.link.rpc("rooms").getJSONObject("data")
            val a = d.optJSONArray("mine")
            mine = if (a == null) emptyList() else (0 until a.length()).map { a.getJSONObject(it) }
            failed = null
            runCatching {
                val l = app.link.rpc("rooms_live").getJSONArray("data")
                live = (0 until l.length()).map { l.getJSONObject(it) }
            }
            app.rooms.refresh()
        } catch (e: Exception) {
            failed = if (!app.store.linked) "Сначала подключите компьютер: «Ещё», «Подключить компьютер»." else (e.message ?: "Не получилось") + ". Нужен включённый SC Desk на компьютере."
        }
    }
    // goes into the room and listens to it here
    fun enter(op: String, args: JSONObject) {
        scope.launch {
            note = try {
                app.link.rpc(op, args)
                delay(800)
                app.rooms.refresh()
                app.rooms.follow()
                null
            } catch (e: Exception) { e.message }
            tick++
        }
    }

    Column(Modifier.fillMaxSize().background(C.bg).statusBarsPadding().navigationBarsPadding().pressable { }) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(44.dp).pressable(onBack), contentAlignment = Alignment.Center) { Icon(Ico.Back, 26.dp) }
            Text("Комнаты", color = C.text, fontSize = 24.sp, fontWeight = FontWeight.Bold, modifier = Modifier.weight(1f))
            Row(Modifier.clip(RoundedCornerShape(16.dp)).background(C.accent).pressable { create = true }.padding(horizontal = 16.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
                SIcon("plus", C.bg)
                Text("Создать", color = C.bg, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(start = 6.dp))
            }
        }
        failed?.let { Text(it, color = C.accent, fontSize = 14.sp, modifier = Modifier.padding(20.dp)) }
        note?.let { Text(it, color = C.accent, fontSize = 13.sp, modifier = Modifier.padding(horizontal = 20.dp, vertical = 4.dp)) }
        if (mine == null && failed == null) Text("Спрашиваем компьютер…", color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(20.dp))
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 96.dp)) {
            val list = mine ?: emptyList()
            if (list.isNotEmpty()) item { Text("Мои комнаты", color = C.muted, fontSize = 12.sp, modifier = Modifier.padding(start = 20.dp, top = 10.dp, bottom = 6.dp)) }
            items(list, key = { "m" + it.optString("channel") }) { r ->
                val active = r.optBoolean("active")
                val here = inRoom?.channel == r.optString("channel")
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 5.dp).clip(RoundedCornerShape(16.dp)).background(if (here) C.warm else C.panel)
                        .pressable {
                            if (here && !following) app.rooms.follow()
                            else if (!here) { if (r.optBoolean("locked")) ask = r else enter("room_enter", JSONObject().put("channel", r.optString("channel"))) }
                        }.padding(14.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Box(Modifier.size(56.dp).clip(RoundedCornerShape(12.dp)).background(C.panel2), contentAlignment = Alignment.Center) { Text(r.optString("name").take(1).uppercase(), color = C.accent, fontSize = 20.sp, fontWeight = FontWeight.Bold) }
                    Column(Modifier.padding(start = 14.dp).weight(1f)) {
                        Text(r.optString("name"), color = C.text, fontSize = 17.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        val cur = r.optString("current").takeIf { it.isNotEmpty() && it != "null" }
                        val online = r.optLong("online")
                        Text(
                            if (active || online > 0) (cur ?: "в комнате") + " · " + online + " чел." else "пусто · очередь " + r.optInt("queue"),
                            color = if (active || online > 0) C.accent else C.muted, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis,
                        )
                    }
                    if (here && following) SIcon("sound", C.accent) else if (r.optBoolean("locked")) SIcon("lock", C.muted)
                }
            }
            if (live.isNotEmpty()) item { Text("Друзья в комнатах", color = C.muted, fontSize = 12.sp, modifier = Modifier.padding(start = 20.dp, top = 16.dp, bottom = 6.dp)) }
            items(live, key = { "l" + it.optString("channel") }) { r ->
                Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 7.dp), verticalAlignment = Alignment.CenterVertically) {
                    Box(Modifier.size(46.dp).clip(CircleShape).background(C.panel2), contentAlignment = Alignment.Center) { Text(r.optString("host").take(1).uppercase(), color = C.text, fontSize = 17.sp, fontWeight = FontWeight.SemiBold) }
                    Column(Modifier.padding(start = 12.dp).weight(1f)) {
                        Text(r.optString("host"), color = C.text, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(r.optString("name") + " · " + r.optInt("members") + " чел.", color = C.muted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    Text("Зайти", color = C.accent, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.clip(RoundedCornerShape(14.dp)).border(1.dp, C.accent, RoundedCornerShape(14.dp)).pressable {
                        if (r.optBoolean("locked")) ask = r else enter("room_join", JSONObject().put("link", r.optString("channel")).put("password", ""))
                    }.padding(horizontal = 18.dp, vertical = 9.dp))
                }
            }
            item {
                Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 14.dp).clip(RoundedCornerShape(16.dp)).background(C.panel).pressable { byLink = true }.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    SIcon("link", C.text)
                    Text("Войти по ссылке или коду", color = C.text, fontSize = 15.sp, fontWeight = FontWeight.Medium, modifier = Modifier.padding(start = 14.dp).weight(1f))
                    SIcon("chev", C.muted)
                }
            }
        }
    }

    // a room with a password
    ask?.let { r ->
        var pw by remember(r) { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { ask = null }, containerColor = C.panel,
            title = { Text(r.optString("name"), color = C.text) },
            text = { Column { Text("В комнате пароль", color = C.muted, fontSize = 13.sp); Field(pw, "Пароль", { pw = it }, secret = true) } },
            confirmButton = {
                Text("Зайти", color = C.accent, fontWeight = FontWeight.SemiBold, modifier = Modifier.pressable {
                    ask = null
                    val mineRoom = (mine ?: emptyList()).any { it.optString("channel") == r.optString("channel") }
                    if (mineRoom) enter("room_enter", JSONObject().put("channel", r.optString("channel")))
                    else enter("room_join", JSONObject().put("link", r.optString("channel")).put("password", pw))
                }.padding(12.dp))
            },
            dismissButton = { Text("Отмена", color = C.muted, modifier = Modifier.pressable { ask = null }.padding(12.dp)) },
        )
    }
    if (byLink) {
        var link by remember { mutableStateOf("") }
        var pw by remember { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { byLink = false }, containerColor = C.panel,
            title = { Text("Войти в комнату", color = C.text) },
            text = { Column { Field(link, "Ссылка или код", { link = it }); Field(pw, "Пароль, если есть", { pw = it }, secret = true) } },
            confirmButton = { Text("Зайти", color = C.accent, fontWeight = FontWeight.SemiBold, modifier = Modifier.pressable { byLink = false; if (link.isNotBlank()) enter("room_join", JSONObject().put("link", link.trim()).put("password", pw)) }.padding(12.dp)) },
            dismissButton = { Text("Отмена", color = C.muted, modifier = Modifier.pressable { byLink = false }.padding(12.dp)) },
        )
    }
    if (create) {
        var name by remember { mutableStateOf("") }
        var pw by remember { mutableStateOf("") }
        var friends by remember { mutableStateOf(false) }
        var everyone by remember { mutableStateOf(false) }
        AlertDialog(
            onDismissRequest = { create = false }, containerColor = C.panel,
            title = { Text("Новая комната", color = C.text) },
            text = {
                Column {
                    Field(name, "Название", { name = it })
                    Field(pw, "Пароль (можно без него)", { pw = it }, secret = true)
                    Toggle("Только для друзей", "иначе войти можно по ссылке", friends) { friends = it }
                    Toggle("Очередь у всех", "иначе треки добавляет только хозяин", everyone) { everyone = it }
                }
            },
            confirmButton = {
                Text("Создать", color = C.accent, fontWeight = FontWeight.SemiBold, modifier = Modifier.pressable {
                    create = false
                    enter("room_create", JSONObject().put("name", name).put("password", pw).put("access", if (friends) "friends" else "link").put("queue_everyone", everyone))
                }.padding(12.dp))
            },
            dismissButton = { Text("Отмена", color = C.muted, modifier = Modifier.pressable { create = false }.padding(12.dp)) },
        )
    }
}
