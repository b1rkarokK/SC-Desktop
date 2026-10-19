package dev.scdesk.app

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** «Настройки» as in the design: a list of sections, each opening its own page; «Внешний вид» has the theme cards, the accent, the density. */
@Composable
internal fun SettingsScreen(app: App, onEq: () -> Unit, onUnlinked: () -> Unit, onPage: (String) -> Unit, onSignOut: () -> Unit, onCheckUpdate: () -> Unit, updateMsg: String?, onBack: () -> Unit) {
    var sub by remember { mutableStateOf<String?>(null) }
    androidx.activity.compose.BackHandler(enabled = sub != null) { sub = null }
    val ctx = androidx.compose.ui.platform.LocalContext.current
    val servers by app.vpnServers.collectAsState()
    when (sub) {
        null -> Page(onBack, "Настройки") {
            Column(Modifier.verticalScroll(rememberScrollState()).padding(horizontal = 16.dp)) {
                Row1("user", "Аккаунт", if (app.store.signedIn) app.store.userName.ifEmpty { "SoundCloud" } else "Вы не вошли") { sub = "account" }
                Row1("palette", "Внешний вид", "Тема, акцент, плотность") { sub = "look" }
                Row1("sound", "Звук", "Эквалайзер, тише ночью") { sub = "sound" }
                Row1("wifi", "Сеть", Tunnel.selected.takeIf { it >= 0 && it < servers.size }?.let { "Через сервер: " + servers[it].name } ?: "Прямое подключение") { sub = "net" }
                Row1("rooms", "Комнаты", "Слушать вместе, через компьютер") { onPage("rooms") }
                Row1("download", "Скачанное", "Лимит ${app.store.downloadLimitGb} ГБ") { sub = "downloads" }
                Row1("info", "О программе", "Версия ${BuildConfig.VERSION_NAME}") { sub = "about" }
            }
        }
        "account" -> Page({ sub = null }, "Аккаунт") {
            Column(Modifier.padding(16.dp)) {
                Text(if (app.store.signedIn) app.store.userName.ifEmpty { "Аккаунт SoundCloud" } else "Вы не вошли", color = C.text, fontSize = 20.sp, fontWeight = FontWeight.SemiBold)
                Text(if (app.store.signedIn) "аккаунт SoundCloud" else "Войти можно в «Ещё».", color = C.muted, fontSize = 13.sp, modifier = Modifier.padding(top = 4.dp, bottom = 16.dp))
                if (app.store.signedIn) BigButton("Выйти", false) { onSignOut(); onBack() }
            }
        }
        "look" -> Appearance(app) { sub = null }
        "sound" -> Page({ sub = null }, "Звук") {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                Item("Эквалайзер", "Десять полос, настройка общая с компьютером", onEq)
                var quiet by remember { mutableStateOf(app.store.nightQuiet) }
                Toggle("Тише ночью", "С 23:00 до 07:00 новый трек начинается на 60% громкости", quiet) { quiet = it; app.store.nightQuiet = it }
            }
        }
        "downloads" -> Page({ sub = null }, "Скачанное") {
            var gb by remember { mutableIntStateOf(app.store.downloadLimitGb) }
            Column(Modifier.padding(16.dp)) {
                Text("Сколько места можно отдать скачанным трекам", color = C.muted, fontSize = 13.sp, modifier = Modifier.padding(bottom = 10.dp))
                Segments(listOf("1 ГБ", "2 ГБ", "5 ГБ", "10 ГБ"), listOf(1, 2, 5, 10).indexOf(gb).coerceAtLeast(0)) { gb = listOf(1, 2, 5, 10)[it]; app.store.downloadLimitGb = gb }
                var wifi by remember { mutableStateOf(app.store.downloadWifiOnly) }
                Toggle("Только по Wi-Fi", "«Скачать все» не тратит мобильный интернет", wifi) { wifi = it; app.store.downloadWifiOnly = it }
                var counted by remember { mutableStateOf(app.store.statsAnon) }
                Toggle("Считать меня в общей статистике", "Раз в сутки приложение отправляет разработчику случайный номер установки, версию и систему. Без имени, аккаунта и того, что вы слушаете.", counted) { counted = it; app.store.statsAnon = it }
                Text("Качество: как у потока SoundCloud (AAC, если он есть, иначе MP3). Занято: ${fmtSize(Downloads.usedBytes())}.", color = C.faint, fontSize = 12.sp, modifier = Modifier.padding(top = 12.dp))
                Spacer(Modifier.height(16.dp))
                BigButton("Открыть скачанное", false) { onPage("downloads") }
            }
        }
        "net" -> Page({ sub = null }, "Сеть") { Column(Modifier.verticalScroll(rememberScrollState())) { NetworkCard(app) } }
        "about" -> Page({ sub = null }, "О программе") {
            var auto by remember { mutableStateOf(app.store.autoUpdate) }
            var cleaned by remember { mutableStateOf<String?>(null) }
            Column(Modifier.verticalScroll(rememberScrollState())) {
                Item("SC Desk ${BuildConfig.VERSION_NAME}", updateMsg ?: "Проверить обновления сейчас", onCheckUpdate)
                Toggle("Проверять обновления", "При запуске, не чаще двух раз в день", auto) { auto = it; app.store.autoUpdate = it }
                Item("Очистить кэш", cleaned ?: "Обложки и сохранённые ответы SoundCloud") {
                    CoverCache.map.evictAll()
                    app.api.forget()
                    ctx.cacheDir.listFiles()?.filter { it.name != "update" }?.forEach { it.deleteRecursively() }
                    cleaned = "Готово"
                }
                Item("Стереть историю", "Её видите только вы, она хранится на телефоне") { app.store.history = ""; cleaned = "Готово" }
                if (app.store.linked) Item("Отключить компьютер", app.store.pcName.ifEmpty { "SC Desk на компьютере" } + ", вход в аккаунт останется") {
                    app.link.stop()
                    app.store.linkChan = ""; app.store.linkKey = ""; app.store.pcDev = ""; app.store.pcName = ""
                    onUnlinked()
                }
            }
        }
    }
}

@Composable
internal fun Row1(icon: String, title: String, sub: String, onClick: () -> Unit) {
    Row(Modifier.fillMaxWidth().pressable(onClick).padding(vertical = 14.dp), verticalAlignment = Alignment.CenterVertically) {
        SIcon(icon, C.text)
        Column(Modifier.padding(start = 16.dp).weight(1f)) {
            Text(title, color = C.text, fontSize = 16.sp, fontWeight = FontWeight.Medium)
            if (sub.isNotEmpty()) Text(sub, color = C.muted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        SIcon("chev", C.faint)
    }
    Box(Modifier.fillMaxWidth().height(1.dp).background(C.border))
}

/** The line icons of the settings, drawn here (no icon library in the package). */
@Composable
internal fun SIcon(kind: String, color: Color) {
    Canvas(Modifier.size(24.dp)) {
        val w = size.width
        val s = Stroke(width = w * 0.08f, cap = StrokeCap.Round)
        fun p(x: Float, y: Float) = Offset(x * w, y * w)
        when (kind) {
            "user" -> {
                drawCircle(color, w * .17f, p(.5f, .32f), style = s)
                drawArc(color, 200f, 140f, false, p(.2f, .58f), Size(w * .6f, w * .5f), style = s)
            }
            "palette" -> {
                drawCircle(color, w * .4f, p(.5f, .5f), style = s)
                for ((x, y) in listOf(.35f to .38f, .55f to .3f, .66f to .5f)) drawCircle(color, w * .045f, p(x, y))
            }
            "sound" -> {
                val path = Path().apply { moveTo(w * .2f, w * .4f); lineTo(w * .35f, w * .4f); lineTo(w * .52f, w * .26f); lineTo(w * .52f, w * .74f); lineTo(w * .35f, w * .6f); lineTo(w * .2f, w * .6f); close() }
                drawPath(path, color, style = s)
                drawArc(color, -40f, 80f, false, p(.52f, .3f), Size(w * .3f, w * .4f), style = s)
            }
            "wifi" -> {
                for (r in listOf(.42f, .28f, .14f)) drawArc(color, 225f, 90f, false, p(.5f - r, .75f - r), Size(w * r * 2, w * r * 2), style = s)
                drawCircle(color, w * .04f, p(.5f, .72f))
            }
            "eq" -> for ((i, h) in listOf(.35f, .6f, .45f).withIndex()) drawLine(color, p(.28f + i * .22f, .75f), p(.28f + i * .22f, .75f - h * .8f), w * .1f, StrokeCap.Round)
            "moon" -> {
                val moon = Path.combine(androidx.compose.ui.graphics.PathOperation.Difference, Path().apply { addOval(androidx.compose.ui.geometry.Rect(p(.2f, .2f), Size(w * .6f, w * .6f))) }, Path().apply { addOval(androidx.compose.ui.geometry.Rect(p(.4f, .12f), Size(w * .56f, w * .56f))) })
                drawPath(moon, color)
            }
            "rooms" -> {
                drawCircle(color, w * .13f, p(.36f, .34f), style = s)
                drawCircle(color, w * .11f, p(.68f, .38f), style = s)
                drawArc(color, 200f, 140f, false, p(.12f, .5f), Size(w * .48f, w * .4f), style = s)
                drawArc(color, 210f, 120f, false, p(.52f, .52f), Size(w * .36f, w * .32f), style = s)
            }
            "shuffle" -> {
                drawLine(color, p(.2f, .3f), p(.8f, .7f), w * .08f, StrokeCap.Round); drawLine(color, p(.2f, .7f), p(.8f, .3f), w * .08f, StrokeCap.Round)
                drawLine(color, p(.8f, .7f), p(.66f, .7f), w * .08f, StrokeCap.Round); drawLine(color, p(.8f, .7f), p(.8f, .56f), w * .08f, StrokeCap.Round)
                drawLine(color, p(.8f, .3f), p(.66f, .3f), w * .08f, StrokeCap.Round); drawLine(color, p(.8f, .3f), p(.8f, .44f), w * .08f, StrokeCap.Round)
            }
            "repeat" -> {
                drawRoundRect(color, p(.18f, .3f), Size(w * .64f, w * .4f), androidx.compose.ui.geometry.CornerRadius(w * .12f), style = s)
                drawLine(color, p(.6f, .2f), p(.72f, .3f), w * .08f, StrokeCap.Round); drawLine(color, p(.6f, .4f), p(.72f, .3f), w * .08f, StrokeCap.Round)
            }
            "translate" -> {
                drawLine(color, p(.16f, .74f), p(.34f, .26f), w * .08f, StrokeCap.Round); drawLine(color, p(.34f, .26f), p(.52f, .74f), w * .08f, StrokeCap.Round)
                drawLine(color, p(.22f, .58f), p(.46f, .58f), w * .08f, StrokeCap.Round)
                drawLine(color, p(.58f, .34f), p(.86f, .34f), w * .08f, StrokeCap.Round); drawLine(color, p(.72f, .24f), p(.72f, .34f), w * .08f, StrokeCap.Round)
                drawLine(color, p(.64f, .34f), p(.78f, .56f), w * .08f, StrokeCap.Round); drawLine(color, p(.8f, .34f), p(.62f, .56f), w * .08f, StrokeCap.Round)
            }
            "queue" -> {
                drawLine(color, p(.2f, .3f), p(.62f, .3f), w * .08f, StrokeCap.Round); drawLine(color, p(.2f, .5f), p(.62f, .5f), w * .08f, StrokeCap.Round)
                drawLine(color, p(.2f, .7f), p(.42f, .7f), w * .08f, StrokeCap.Round)
                drawCircle(color, w * .07f, p(.68f, .72f), style = s); drawLine(color, p(.75f, .72f), p(.75f, .4f), w * .08f, StrokeCap.Round)
            }
            "radio" -> {
                drawCircle(color, w * .08f, p(.5f, .5f))
                for (r in listOf(.2f, .34f)) {
                    drawArc(color, -50f, 100f, false, p(.5f - r, .5f - r), Size(w * r * 2, w * r * 2), style = s)
                    drawArc(color, 130f, 100f, false, p(.5f - r, .5f - r), Size(w * r * 2, w * r * 2), style = s)
                }
            }
            "history" -> {
                drawCircle(color, w * .36f, p(.5f, .5f), style = s)
                drawLine(color, p(.5f, .3f), p(.5f, .5f), w * .08f, StrokeCap.Round); drawLine(color, p(.5f, .5f), p(.64f, .58f), w * .08f, StrokeCap.Round)
            }
            "chart" -> for ((i, h) in listOf(.3f, .5f, .4f).withIndex()) drawLine(color, p(.3f + i * .2f, .78f), p(.3f + i * .2f, .78f - h), w * .1f, StrokeCap.Round)
            "gear" -> {
                drawCircle(color, w * .15f, p(.5f, .5f), style = s)
                for (k in 0 until 8) {
                    val a = Math.toRadians(k * 45.0)
                    drawLine(color, p(.5f + (.27f * Math.cos(a)).toFloat(), .5f + (.27f * Math.sin(a)).toFloat()), p(.5f + (.37f * Math.cos(a)).toFloat(), .5f + (.37f * Math.sin(a)).toFloat()), w * .09f, StrokeCap.Round)
                }
            }
            "bell" -> {
                drawArc(color, 180f, 180f, false, p(.26f, .18f), Size(w * .48f, w * .46f), style = s)
                drawLine(color, p(.26f, .41f), p(.22f, .7f), w * .08f, StrokeCap.Round); drawLine(color, p(.74f, .41f), p(.78f, .7f), w * .08f, StrokeCap.Round)
                drawLine(color, p(.2f, .72f), p(.8f, .72f), w * .08f, StrokeCap.Round); drawCircle(color, w * .05f, p(.5f, .84f))
            }
            "search" -> {
                drawCircle(color, w * .22f, p(.44f, .44f), style = s); drawLine(color, p(.6f, .6f), p(.8f, .8f), w * .08f, StrokeCap.Round)
            }
            "refresh" -> {
                drawArc(color, 40f, 290f, false, p(.2f, .2f), Size(w * .6f, w * .6f), style = s)
                drawLine(color, p(.74f, .26f), p(.78f, .12f), w * .08f, StrokeCap.Round); drawLine(color, p(.74f, .26f), p(.88f, .3f), w * .08f, StrokeCap.Round)
            }
            "dislike" -> {
                drawCircle(color, w * .32f, p(.5f, .5f), style = s); drawLine(color, p(.28f, .72f), p(.72f, .28f), w * .08f, StrokeCap.Round)
            }
            "headphones" -> {
                drawArc(color, 180f, 180f, false, p(.2f, .2f), Size(w * .6f, w * .6f), style = s)
                drawRoundRect(color, p(.16f, .5f), Size(w * .14f, w * .26f), androidx.compose.ui.geometry.CornerRadius(w * .05f), style = s)
                drawRoundRect(color, p(.7f, .5f), Size(w * .14f, w * .26f), androidx.compose.ui.geometry.CornerRadius(w * .05f), style = s)
            }
            "download" -> {
                drawLine(color, p(.5f, .2f), p(.5f, .62f), w * .08f, StrokeCap.Round)
                drawLine(color, p(.34f, .48f), p(.5f, .64f), w * .08f, StrokeCap.Round); drawLine(color, p(.66f, .48f), p(.5f, .64f), w * .08f, StrokeCap.Round)
                drawLine(color, p(.22f, .8f), p(.78f, .8f), w * .08f, StrokeCap.Round)
            }
            "lock" -> {
                drawRoundRect(color, p(.26f, .44f), Size(w * .48f, w * .36f), androidx.compose.ui.geometry.CornerRadius(w * .07f), style = s)
                drawArc(color, 180f, 180f, false, p(.34f, .22f), Size(w * .32f, w * .4f), style = s)
            }
            "link" -> {
                drawRoundRect(color, p(.12f, .38f), Size(w * .42f, w * .24f), androidx.compose.ui.geometry.CornerRadius(w * .12f), style = s)
                drawRoundRect(color, p(.46f, .38f), Size(w * .42f, w * .24f), androidx.compose.ui.geometry.CornerRadius(w * .12f), style = s)
            }
            "plus" -> { drawLine(color, p(.5f, .22f), p(.5f, .78f), w * .09f, StrokeCap.Round); drawLine(color, p(.22f, .5f), p(.78f, .5f), w * .09f, StrokeCap.Round) }
            "mic" -> {
                drawRoundRect(color, p(.38f, .16f), Size(w * .24f, w * .4f), androidx.compose.ui.geometry.CornerRadius(w * .12f), style = s)
                drawArc(color, 0f, 180f, false, p(.26f, .34f), Size(w * .48f, w * .36f), style = s); drawLine(color, p(.5f, .7f), p(.5f, .84f), w * .08f, StrokeCap.Round)
            }
            "close" -> { drawLine(color, p(.28f, .28f), p(.72f, .72f), w * .08f, StrokeCap.Round); drawLine(color, p(.72f, .28f), p(.28f, .72f), w * .08f, StrokeCap.Round) }
            "clock" -> {
                drawCircle(color, w * .34f, p(.5f, .5f), style = s)
                drawLine(color, p(.5f, .32f), p(.5f, .5f), w * .08f, StrokeCap.Round); drawLine(color, p(.5f, .5f), p(.62f, .58f), w * .08f, StrokeCap.Round)
            }
            "dots" -> for (y in listOf(.25f, .5f, .75f)) drawCircle(color, w * .06f, p(.5f, y))
            "chev" -> { drawLine(color, p(.4f, .26f), p(.64f, .5f), w * .08f, StrokeCap.Round); drawLine(color, p(.64f, .5f), p(.4f, .74f), w * .08f, StrokeCap.Round) }
            else -> {
                drawCircle(color, w * .4f, p(.5f, .5f), style = s)
                drawLine(color, p(.5f, .46f), p(.5f, .7f), w * .08f, StrokeCap.Round)
                drawCircle(color, w * .045f, p(.5f, .32f))
            }
        }
    }
}

// ---------------------------------------------------------------- appearance

private val accents = listOf(0xFFFF7B3F, 0xFFEC5B7A, 0xFFB77DEE, 0xFF5B9DF5, 0xFF3FBF9A, 0xFFF5C242, 0xFFE8E8E8)

@Composable
private fun Appearance(app: App, onBack: () -> Unit) {
    var theme by remember { mutableIntStateOf(app.store.theme) }
    var system by remember { mutableStateOf(app.store.followSystem) }
    var accent by remember { mutableIntStateOf(app.store.accent) }
    var cover by remember { mutableStateOf(app.store.accentFromCover) }
    var density by remember { mutableIntStateOf(app.store.density) }
    var square by remember { mutableStateOf(app.store.squareButton) }
    var anim by remember { mutableStateOf(app.store.animations) }
    Page(onBack, "Внешний вид") {
        Column(Modifier.verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).navigationBarsPadding()) {
            Label("ТЕМА")
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                listOf("Тёплая", "OLED", "Светлая").forEachIndexed { i, name ->
                    val on = theme == i
                    val c = C.preview(i)
                    Column(Modifier.weight(1f), horizontalAlignment = Alignment.CenterHorizontally) {
                        Column(
                            Modifier.fillMaxWidth().height(116.dp).clip(RoundedCornerShape(14.dp)).background(c[0]).border(if (on) 2.dp else 1.dp, if (on) C.accent else C.border, RoundedCornerShape(14.dp))
                                .pressable { theme = i; app.store.theme = i; C.mode = i }.padding(10.dp),
                        ) {
                            Box(Modifier.fillMaxWidth(.7f).height(7.dp).clip(RoundedCornerShape(4.dp)).background(c[2]))
                            Spacer(Modifier.height(4.dp))
                            Box(Modifier.fillMaxWidth(.4f).height(5.dp).clip(RoundedCornerShape(3.dp)).background(c[2].copy(alpha = .5f)))
                            Spacer(Modifier.height(8.dp))
                            Box(Modifier.fillMaxWidth().height(40.dp).clip(RoundedCornerShape(8.dp)).background(c[1]).padding(8.dp)) { Box(Modifier.size(16.dp).clip(RoundedCornerShape(4.dp)).background(c[3])) }
                            Spacer(Modifier.height(6.dp))
                            Box(Modifier.fillMaxWidth().height(6.dp).clip(RoundedCornerShape(3.dp)).background(c[1]))
                        }
                        Text(name, color = if (on) C.accent else C.muted, fontSize = 12.sp, fontWeight = FontWeight.Medium, modifier = Modifier.padding(top = 6.dp))
                    }
                }
            }
            Spacer(Modifier.height(10.dp))
            Box(Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(C.panel)) {
                Toggle("Как в системе", "светлая днём, тёмная ночью", system) { system = it; app.store.followSystem = it; C.followSystem = it }
            }
            Label("АКЦЕНТ")
            Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(C.panel)) {
                Row(Modifier.padding(14.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    accents.forEachIndexed { i, hex ->
                        Box(Modifier.size(32.dp).clip(CircleShape).background(Color(hex)).border(if (accent == i && !cover) 3.dp else 0.dp, C.text, CircleShape).pressable { accent = i; app.store.accent = i; C.accentIdx = i; cover = false; app.store.accentFromCover = false; C.useCover = false })
                    }
                }
                Toggle("Из обложки играющего трека", "акцент подстраивается под картинку", cover) { cover = it; app.store.accentFromCover = it; C.useCover = it }
            }
            Label("ПЛОТНОСТЬ СПИСКОВ")
            Segments(listOf("Компактно", "Обычно", "Крупно"), density) { density = it; app.store.density = it; C.density = it }
            Label("СКРУГЛЕНИЕ УГЛОВ")
            var corners by remember { mutableIntStateOf(app.store.corners) }
            Segments(listOf("Острые", "Обычные", "Круглые"), corners) { corners = it; app.store.corners = it; C.corners = it }
            Label("СВОИ ЦВЕТА")
            CustomColors(app)
            Label("КНОПКА ПЛЕЕРА")
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                listOf(true to "Квадрат", false to "Круг").forEach { (sq, name) ->
                    val on = square == sq
                    Column(Modifier.weight(1f), horizontalAlignment = Alignment.CenterHorizontally) {
                        Box(
                            Modifier.fillMaxWidth().height(76.dp).clip(RoundedCornerShape(14.dp)).background(C.panel).border(if (on) 2.dp else 1.dp, if (on) C.accent else C.border, RoundedCornerShape(14.dp))
                                .pressable { square = sq; app.store.squareButton = sq; C.squareButton = sq },
                            contentAlignment = Alignment.Center,
                        ) {
                            Box(Modifier.size(46.dp).clip(if (sq) RoundedCornerShape(14.dp) else CircleShape).background(C.text), contentAlignment = Alignment.Center) { Icon(Ico.Pause, 26.dp, C.bg) }
                        }
                        Text(name, color = if (on) C.accent else C.muted, fontSize = 12.sp, modifier = Modifier.padding(top = 6.dp))
                    }
                }
            }
            Spacer(Modifier.height(10.dp))
            Box(Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(C.panel)) {
                Toggle("Анимации", "меньше, если нужно сберечь батарею", anim) { anim = it; app.store.animations = it; C.anim = it }
            }
            Spacer(Modifier.height(24.dp))
        }
    }
}

/** Own colours instead of a theme: the hex of the background, the text and the accent; a wrong hex is not taken. */
@Composable
private fun CustomColors(app: App) {
    fun hex(c: Color?) = c?.let { "%06X".format(it.toArgb() and 0xFFFFFF) } ?: ""
    var bg by remember { mutableStateOf(hex(C.customBg)) }
    var tx by remember { mutableStateOf(hex(C.customText)) }
    var ac by remember { mutableStateOf(hex(C.customAccent)) }
    fun parse(s: String): Color? = s.trim().removePrefix("#").takeIf { it.length == 6 }?.toLongOrNull(16)?.let { Color(0xFF000000 or it) }
    fun apply() {
        val b = parse(bg)
        val t = parse(tx)
        if (b != null && t != null) {
            C.customBg = b; C.customText = t; C.customAccent = parse(ac)
            app.store.customBg = b.toArgb(); app.store.customText = t.toArgb(); app.store.customAccent = parse(ac)?.toArgb() ?: 0
        }
    }
    Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(C.panel).padding(14.dp)) {
        Text("Вставьте цвета в виде 16-ричных кодов: фон, текст и акцент. Нужны фон и текст.", color = C.muted, fontSize = 12.sp, modifier = Modifier.padding(bottom = 8.dp))
        listOf(Triple("Фон", bg, { v: String -> bg = v }), Triple("Текст", tx, { v: String -> tx = v }), Triple("Акцент", ac, { v: String -> ac = v })).forEach { (label, value, set) ->
            Row(Modifier.padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(label, color = C.text, fontSize = 14.sp, modifier = Modifier.width(72.dp))
                Box(Modifier.size(22.dp).clip(RoundedCornerShape(6.dp)).background(parse(value) ?: C.panel2).border(1.dp, C.border, RoundedCornerShape(6.dp)))
                Box(Modifier.padding(start = 10.dp).weight(1f).clip(RoundedCornerShape(10.dp)).background(C.panel2).padding(horizontal = 12.dp, vertical = 9.dp)) {
                    if (value.isEmpty()) Text("#RRGGBB", color = C.faint, fontSize = 14.sp)
                    androidx.compose.foundation.text.BasicTextField(value, { set(it.take(7)); apply() }, singleLine = true, textStyle = androidx.compose.ui.text.TextStyle(color = C.text, fontSize = 14.sp), cursorBrush = androidx.compose.ui.graphics.SolidColor(C.accent), modifier = Modifier.fillMaxWidth())
                }
            }
        }
        Spacer(Modifier.height(6.dp))
        Text(
            "Вернуть обычную тему", color = C.accent, fontSize = 14.sp, fontWeight = FontWeight.SemiBold,
            modifier = Modifier.pressable {
                C.customBg = null; C.customText = null; C.customAccent = null
                app.store.customBg = 0; app.store.customText = 0; app.store.customAccent = 0
                bg = ""; tx = ""; ac = ""
            }.padding(vertical = 8.dp),
        )
    }
}

@Composable
private fun Label(text: String) {
    Text(text, color = C.muted, fontSize = 11.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(top = 18.dp, bottom = 8.dp))
}

@Composable
internal fun Segments(labels: List<String>, selected: Int, onPick: (Int) -> Unit) {
    Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(C.panel).padding(3.dp)) {
        labels.forEachIndexed { i, l ->
            Text(
                l, color = if (i == selected) C.text else C.muted, fontSize = 13.sp, fontWeight = FontWeight.Medium, textAlign = TextAlign.Center,
                modifier = Modifier.weight(1f).clip(RoundedCornerShape(10.dp)).background(if (i == selected) C.panel2 else Color.Transparent).pressable { onPick(i) }.padding(vertical = 10.dp),
            )
        }
    }
}
