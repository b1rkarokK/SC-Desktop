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
import androidx.compose.material3.Text
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
import org.json.JSONObject

/** A playlist of the user: its number, name and how many tracks it holds. */
private data class MyList(val id: Long, val title: String, val n: Int)

/**
 * «Добавить в плейлист»: the user's own playlists (the computer lists them and does the writing, it holds the session
 * that writes to SoundCloud), a new playlist with this track in it, or a click on a playlist to put the track into it.
 */
@Composable
internal fun PlaylistPicker(app: App, track: Track, onClose: () -> Unit) {
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
        runCatching { app.link.rpc("my_playlists").getJSONArray("data") }
            .onSuccess { a -> lists = (0 until a.length()).map { a.getJSONObject(it) }.map { MyList(it.getLong("id"), it.optString("title"), it.optInt("n")) } }
            .onFailure { lists = emptyList(); note = "Не получилось: ${it.message}" }
    }

    Box(Modifier.fillMaxSize().background(Color(0xAA000000)).pressable(onClose)) {
        Column(
            Modifier.align(Alignment.BottomCenter).fillMaxWidth().heightIn(max = 520.dp).clip(RoundedCornerShape(topStart = 24.dp, topEnd = 24.dp)).background(C.panel)
                .pressable { }.navigationBarsPadding().padding(top = 16.dp, bottom = 12.dp),
        ) {
            Text("В плейлист", color = C.text, fontSize = 18.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(start = 20.dp))
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
                } else if (app.store.linked) {
                    Text("＋ Новый плейлист", color = C.accent, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.fillMaxWidth().pressable { creating = true }.padding(horizontal = 20.dp, vertical = 12.dp))
                }
                when (val l = lists) {
                    null -> Text("Загружаем…", color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(20.dp))
                    else -> l.forEach { p ->
                        Row(Modifier.fillMaxWidth().pressable {
                            if (busy) return@pressable
                            busy = true
                            scope.launch {
                                runCatching { app.link.rpc("pl_add", JSONObject().put("playlist", p.id).put("track", track.id)) }
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
