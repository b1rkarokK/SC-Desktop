package dev.scdesk.app

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch
import org.json.JSONObject

/**
 * «Импорт» of a playlist from another service (Yandex Music, Spotify and the like), as the computer does it: a link is
 * pasted, the songs are found on SoundCloud and put into a new playlist of the account. The computer does the work
 * (it holds the session that writes to SoundCloud), so this needs it to be connected.
 */
@Composable
internal fun ImportPlaylist(app: App) {
    val scope = rememberCoroutineScope()
    var open by remember { mutableStateOf(false) }
    var input by remember { mutableStateOf("") }
    var found by remember { mutableStateOf<Pair<String, Int>?>(null) }
    var note by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }

    Column(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 6.dp)) {
        Text(
            if (open) "Скрыть импорт" else "Импорт плейлиста из другого сервиса",
            color = C.accent, fontSize = 14.sp, fontWeight = FontWeight.SemiBold,
            modifier = Modifier.pressable { open = !open }.padding(vertical = 6.dp),
        )
        if (!open) return@Column
        if (!app.store.linked) {
            Text("Нужен подключённый компьютер: импорт делает он. Подключите его в «Ещё».", color = C.muted, fontSize = 13.sp)
            return@Column
        }
        Text("Вставьте ссылку на плейлист (Яндекс Музыка, Spotify и другие).", color = C.muted, fontSize = 13.sp)
        Box(Modifier.padding(top = 8.dp).fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(C.panel2).padding(horizontal = 14.dp, vertical = 12.dp)) {
            if (input.isEmpty()) Text("https://…", color = C.faint, fontSize = 14.sp)
            BasicTextField(input, { input = it; found = null; note = null }, singleLine = true, textStyle = TextStyle(color = C.text, fontSize = 14.sp), cursorBrush = SolidColor(C.accent), modifier = Modifier.fillMaxWidth())
        }
        Spacer(Modifier.height(8.dp))
        val f = found
        if (f == null) {
            BigButton(if (busy) "Смотрим…" else "Проверить ссылку", false) {
                if (busy || input.isBlank()) return@BigButton
                busy = true
                scope.launch {
                    runCatching { app.link.rpc("import_preview", JSONObject().put("input", input.trim())).getJSONObject("data") }
                        .onSuccess { found = it.optString("name") to it.optInt("n"); note = null }
                        .onFailure { note = "Не вышло: ${it.message}" }
                    busy = false
                }
            }
        } else {
            Text("«${f.first}»: песен ${f.second}", color = C.text, fontSize = 15.sp, modifier = Modifier.padding(bottom = 8.dp))
            BigButton(if (busy) "Переносим… это может занять минуту" else "Импортировать", false) {
                if (busy) return@BigButton
                busy = true
                note = "Ищем песни на SoundCloud, не закрывайте приложение"
                scope.launch {
                    runCatching { app.link.rpc("import_run", JSONObject().put("input", input.trim()).put("title", f.first).put("private", false), waitMs = 300_000).getJSONObject("data") }
                        .onSuccess {
                            note = "Готово: найдено ${it.optInt("found")} из ${it.optInt("total")}" + (if (it.optInt("playlists") > 1) ", плейлистов: ${it.optInt("playlists")}" else "")
                            input = ""; found = null
                        }
                        .onFailure { note = "Не вышло: ${it.message}" }
                    busy = false
                }
            }
        }
        note?.let { Text(it, color = C.faint, fontSize = 12.sp, modifier = Modifier.padding(top = 8.dp)) }
    }
}
