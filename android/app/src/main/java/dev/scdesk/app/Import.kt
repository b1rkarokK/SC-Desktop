package dev.scdesk.app

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.BasicTextField
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
import kotlinx.coroutines.withContext
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
                    val own = runCatching { withContext(kotlinx.coroutines.Dispatchers.IO) { Independent.readSource(input) } }
                    val pc = if (own.isSuccess || !app.store.linked) null else runCatching { app.link.rpc("import_preview", JSONObject().put("input", input.trim())).getJSONObject("data") }
                    (if (own.isSuccess) Result.success(JSONObject().put("name", own.getOrThrow().first).put("n", own.getOrThrow().second.size)) else pc ?: Result.failure(own.exceptionOrNull() ?: java.io.IOException("Не вышло")))
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
                    val own = runCatching { withContext(kotlinx.coroutines.Dispatchers.IO) { Independent.importPlaylist(app, input, f.first) { d, n -> note = "Ищем песни на SoundCloud: $d из $n" } } }
                    val res = if (own.isSuccess) Result.success(own.getOrThrow().second to own.getOrThrow().third)
                    else if (app.store.linked) runCatching { app.link.rpc("import_run", JSONObject().put("input", input.trim()).put("title", f.first).put("private", false), waitMs = 300_000).getJSONObject("data").let { it.optInt("found") to it.optInt("total") } }
                    else Result.failure(own.exceptionOrNull() ?: java.io.IOException("Не вышло"))
                    res
                        .onSuccess { (gotN, allN) ->
                            note = "Готово: найдено $gotN из $allN"
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

/**
 * «В очереди»: likes (and unlikes) that SoundCloud has not taken yet. The computer keeps the queue; here the number is
 * shown with «Отправить сейчас». When SoundCloud wants a check that the user is not a robot, the computer must show it.
 */
@Composable
internal fun PendingLikes(app: App, account: Int) {
    val scope = rememberCoroutineScope()
    var items by remember(account) { mutableStateOf<List<Pair<String, Boolean>>>(emptyList()) }
    var check by remember(account) { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var note by remember { mutableStateOf<String?>(null) }
    suspend fun load() {
        if (!app.store.linked) return
        runCatching { app.link.rpc("likes_pending", waitMs = 4_000).getJSONObject("data") }.onSuccess { d ->
            val a = d.getJSONArray("items")
            items = (0 until a.length()).map { a.getJSONObject(it) }.map { it.getJSONObject("t").optString("title") to it.optBoolean("liked") }
            check = d.optBoolean("check")
        }
    }
    androidx.compose.runtime.LaunchedEffect(account) { load() }
    if (items.isEmpty()) return
    Column(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 6.dp).clip(RoundedCornerShape(14.dp)).background(C.panel).padding(14.dp)) {
        Text("В очереди: ${items.size}", color = C.text, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
        Text(
            if (check) "SoundCloud просит подтвердить, что вы не робот. Нажмите «Пройти проверку на компьютере»: откроется окно SoundCloud." else "SoundCloud пока не принял эти лайки. Программа повторит сама, можно отправить сейчас.",
            color = C.muted, fontSize = 13.sp, modifier = Modifier.padding(top = 4.dp),
        )
        items.take(5).forEach { (title, liked) -> Text((if (liked) "♥ " else "✕ ") + title, color = C.faint, fontSize = 12.sp, maxLines = 1, modifier = Modifier.padding(top = 2.dp)) }
        Spacer(Modifier.height(8.dp))
        Text(
            if (busy) "Отправляем…" else if (check) "Отправить сейчас (на компьютере откроется проверка)" else "Отправить сейчас", color = C.accent, fontSize = 14.sp, fontWeight = FontWeight.SemiBold,
            modifier = Modifier.pressable {
                if (busy) return@pressable
                busy = true
                scope.launch {
                    runCatching { app.link.rpc("likes_flush", waitMs = 120_000).optInt("data") }
                        .onSuccess { left -> note = if (left == 0) "Все лайки отправлены" else "SoundCloud пока не принял: $left"; load() }
                        .onFailure { note = "Не вышло: ${it.message}" }
                    busy = false
                }
            }.padding(vertical = 6.dp),
        )
        note?.let { Text(it, color = C.faint, fontSize = 12.sp) }
    }
}

/** «Новый плейлист» of the likes list: a name, and the computer makes it (it holds the session that writes to SoundCloud). */
@Composable
internal fun NewPlaylist(app: App) {
    val scope = rememberCoroutineScope()
    val ctx = androidx.compose.ui.platform.LocalContext.current
    var open by remember { mutableStateOf(false) }
    var name by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    Column(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 6.dp)) {
        Text(if (open) "Скрыть" else "＋ Новый плейлист", color = C.accent, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.pressable { open = !open }.padding(vertical = 6.dp))
        if (!open) return@Column
        Box(Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(C.panel2).padding(horizontal = 14.dp, vertical = 12.dp)) {
            if (name.isEmpty()) Text("Название плейлиста", color = C.faint, fontSize = 14.sp)
            BasicTextField(name, { name = it }, singleLine = true, textStyle = TextStyle(color = C.text, fontSize = 14.sp), cursorBrush = SolidColor(C.accent), modifier = Modifier.fillMaxWidth())
        }
        Spacer(Modifier.height(8.dp))
        BigButton(if (busy) "Создаём…" else "Создать", false) {
            if (busy || name.isBlank()) return@BigButton
            busy = true
            scope.launch {
                runCatching { Writes.call(app, "pl_create", JSONObject().put("title", name.trim())) }
                    .onSuccess { android.widget.Toast.makeText(ctx, L.t("Плейлист «{0}» создан").replace("{0}", name.trim()), android.widget.Toast.LENGTH_SHORT).show(); name = ""; open = false }
                    .onFailure { android.widget.Toast.makeText(ctx, L.t("Не получилось: {0}").replace("{0}", it.message ?: ""), android.widget.Toast.LENGTH_LONG).show() }
                busy = false
            }
        }
    }
}
