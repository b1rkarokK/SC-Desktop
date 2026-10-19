package dev.scdesk.app

import android.Manifest
import android.content.pm.PackageManager
import android.os.SystemClock
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.MultiFormatReader
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.util.concurrent.Executors

// ----------------------------------------------------------------------- QR

@Composable
fun ScanScreen(app: App, initial: String?, onBack: (() -> Unit)? = null, onLogin: (() -> Unit)? = null, onDone: () -> Unit) {
    val ctx = LocalContext.current
    var allowed by remember { mutableStateOf(ContextCompat.checkSelfPermission(ctx, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { allowed = it }
    var status by remember { mutableStateOf("Наведите камеру на QR-код в SC Desk на компьютере") }
    var pairing by remember { mutableStateOf<Pairing?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(Unit) { if (!allowed) ask.launch(Manifest.permission.CAMERA) }
    DisposableEffect(Unit) { onDispose { pairing?.cancel() } }

    // the code was read (by the camera here, or by the system, which opened the app with the link)
    fun begin(text: String) {
        val code = PairCode.parse(text)
        if (code != null && pairing == null) {
            status = if (code.take) "Передаём вход на компьютер. Подтвердите его там…" else "Ждём подтверждения на компьютере…"
            error = null
            val p = Pairing(app.store, code) { r ->
                r.onSuccess { onDoneOnMain(ctx) { onDone() } }
                r.onFailure { e ->
                    onDoneOnMain(ctx) {
                        error = e.message ?: "Не вышло"
                        pairing = null
                        status = "Наведите камеру на QR-код в SC Desk на компьютере"
                    }
                }
            }
            pairing = p
            p.start()
        }
    }
    LaunchedEffect(initial) { initial?.let(::begin) }

    Column(Modifier.fillMaxSize().background(C.bg).statusBarsPadding().navigationBarsPadding().padding(16.dp).consumeTaps()) {
        Row(Modifier.padding(top = 8.dp, bottom = 16.dp), verticalAlignment = Alignment.CenterVertically) {
            if (onBack != null) Box(Modifier.size(44.dp).pressable(onBack), contentAlignment = Alignment.CenterStart) { Icon(Ico.Back, 26.dp) }
            Text("Вход по QR-коду", color = C.text, fontSize = 22.sp, fontWeight = FontWeight.SemiBold)
        }
        Box(Modifier.fillMaxWidth().aspectRatio(1f).clip(RoundedCornerShape(20.dp)).background(C.panel).border(1.dp, C.border, RoundedCornerShape(20.dp))) {
            if (allowed && pairing == null) {
                CameraPreview(Modifier.fillMaxSize()) { text -> onDoneOnMain(ctx) { begin(text) } }
            } else if (!allowed) {
                Column(Modifier.fillMaxSize().padding(24.dp), verticalArrangement = Arrangement.Center, horizontalAlignment = Alignment.CenterHorizontally) {
                    Text("Нужен доступ к камере, чтобы прочитать QR-код", color = C.muted, fontSize = 14.sp)
                    Spacer(Modifier.height(12.dp))
                    Text("Разрешить", color = C.bg, modifier = Modifier.clip(RoundedCornerShape(16.dp)).background(C.text).padding(horizontal = 20.dp, vertical = 12.dp).pressable { ask.launch(Manifest.permission.CAMERA) })
                }
            } else {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Text("Подключаемся…", color = C.muted) }
            }
            androidx.compose.foundation.Canvas(Modifier.fillMaxSize().padding(36.dp)) {
                val len = size.width * .16f
                val w = 5f
                val col = C.accent
                fun corner(x: Float, y: Float, dx: Float, dy: Float) {
                    drawLine(col, androidx.compose.ui.geometry.Offset(x, y), androidx.compose.ui.geometry.Offset(x + dx * len, y), w, androidx.compose.ui.graphics.StrokeCap.Round)
                    drawLine(col, androidx.compose.ui.geometry.Offset(x, y), androidx.compose.ui.geometry.Offset(x, y + dy * len), w, androidx.compose.ui.graphics.StrokeCap.Round)
                }
                corner(0f, 0f, 1f, 1f); corner(size.width, 0f, -1f, 1f); corner(0f, size.height, 1f, -1f); corner(size.width, size.height, -1f, -1f)
            }
        }
        Text(status, color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(top = 16.dp))
        error?.let { Text(it, color = C.accent, fontSize = 14.sp, modifier = Modifier.padding(top = 8.dp)) }
        Text("На компьютере: Настройки, Интеграции, «Подключить телефон» (или «Войти по QR с телефона», если там ещё нет входа).", color = C.faint, fontSize = 12.sp, modifier = Modifier.padding(top = 8.dp))
        Spacer(Modifier.weight(1f))
        // the code by hand: the link of the QR (as text) can be pasted
        Text(
            "Ввести код вручную", color = C.text, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).border(1.dp, C.border, RoundedCornerShape(16.dp)).pressable {
                val clip = ctx.getSystemService(android.content.ClipboardManager::class.java).primaryClip?.getItemAt(0)?.text?.toString().orEmpty()
                if (PairCode.parse(clip) != null) begin(clip) else error = "Скопируйте ссылку с QR-кода (scdesk://pair…) и нажмите ещё раз"
            }.padding(vertical = 14.dp),
        )
        if (onLogin != null) {
            Spacer(Modifier.height(8.dp))
            BigButton("Войти через SoundCloud", true) { onLogin() }
        }
    }
}

/** The pairing answers on a worker thread: the screen changes on the main one. */
private fun onDoneOnMain(ctx: android.content.Context, block: () -> Unit) {
    android.os.Handler(ctx.mainLooper).post(block)
}

@Composable
private fun CameraPreview(modifier: Modifier, onText: (String) -> Unit) {
    val owner = LocalLifecycleOwner.current
    val executor = remember { Executors.newSingleThreadExecutor() }
    val reader = remember {
        MultiFormatReader().apply { setHints(mapOf(DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE), DecodeHintType.TRY_HARDER to true)) }
    }
    DisposableEffect(Unit) { onDispose { executor.shutdown() } }
    AndroidView(modifier = modifier, factory = { c ->
        PreviewView(c).also { view ->
            val future = ProcessCameraProvider.getInstance(c)
            future.addListener({
                val provider = future.get()
                val preview = Preview.Builder().build().also { it.setSurfaceProvider(view.surfaceProvider) }
                val analysis = ImageAnalysis.Builder().setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST).build()
                analysis.setAnalyzer(executor) { img ->
                    try {
                        val plane = img.planes[0]
                        val bytes = ByteArray(plane.buffer.remaining()).also { plane.buffer.get(it) }
                        val src = PlanarYUVLuminanceSource(bytes, plane.rowStride, img.height, 0, 0, img.width, img.height, false)
                        val result = reader.decodeWithState(BinaryBitmap(HybridBinarizer(src)))
                        onText(result.text)
                    } catch (e: Exception) {
                        // no code in this frame
                    } finally {
                        reader.reset()
                        img.close()
                    }
                }
                provider.unbindAll()
                provider.bindToLifecycle(owner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis)
            }, ContextCompat.getMainExecutor(c))
        }
    })
}

// ------------------------------------------------------------------- shared

/** The device that plays now, if it is not this phone: the one that told "playing" a moment ago. */
internal fun activeRemote(peers: Map<String, Peer>, localPlaying: Boolean, now: Long): Peer? =
    if (localPlaying) null else peers.values.filter { it.playing && it.track != null && now - it.seenAt < 25_000 }.maxByOrNull { it.posAt }

private fun fmt(ms: Long): String {
    val s = (ms / 1000).coerceAtLeast(0)
    return "%d:%02d".format(s / 60, s % 60)
}

/** What the mini player and the player show: this phone's track, or the one that plays on another device. */
internal data class Shown(val track: Track?, val playing: Boolean, val remote: Peer?, val loading: Boolean)

@Composable
internal fun MiniPlayer(t: Track, shown: Shown, modifier: Modifier, progress: () -> Float, roomLine: String? = null, onOpen: () -> Unit, onToggle: () -> Unit) {
    val border = if (shown.remote != null || roomLine != null) C.accent else C.border
    Column(modifier.fillMaxWidth().clip(RoundedCornerShape(18.dp)).background(C.panel).border(1.dp, border, RoundedCornerShape(18.dp)).pressable(onOpen)) {
        Row(Modifier.padding(10.dp), verticalAlignment = Alignment.CenterVertically) {
            Cover(t.artwork, 44.dp, 8.dp)
            Column(Modifier.padding(start = 12.dp).weight(1f)) {
                Text(t.title, color = C.text, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(
                    roomLine ?: if (shown.remote != null) L.t("Играет на {0}").replace("{0}", L.t(shown.remote.name.ifEmpty { "компьютере" })) else t.artist,
                    color = if (shown.remote != null || roomLine != null) C.accent else C.muted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis,
                )
            }
            Box(Modifier.size(44.dp).clip(if (C.squareButton) RoundedCornerShape(16.dp) else androidx.compose.foundation.shape.CircleShape).background(C.text).pressable(onToggle), contentAlignment = Alignment.Center) {
                Icon(if (shown.playing) Ico.Pause else Ico.Play, 26.dp, C.bg)
            }
        }
        // the way through the track: a thin line along the bottom of the card
        Box(Modifier.fillMaxWidth().height(2.dp).background(C.panel2)) { Box(Modifier.fillMaxWidth(progress().coerceIn(0f, 1f)).height(2.dp).background(C.accent)) }
    }
}

@Composable
internal fun PlayerScreen(app: App, shown: Shown, t: Track, onClose: () -> Unit, onDevices: () -> Unit, onLyrics: () -> Unit = {}, onEq: () -> Unit = {}, onRooms: () -> Unit = {}) {
    val remote = shown.remote
    val state by app.engine.state.collectAsState()
    var pos by remember { mutableLongStateOf(0L) }
    var dragging by remember { mutableStateOf<Float?>(null) }
    val dur = if (remote != null) t.durationMs else state.durationMs.takeIf { it > 0 } ?: t.durationMs
    // the position counts on (about twice a second) only while this screen is open
    LaunchedEffect(remote?.dev, shown.playing, t.id) {
        while (true) {
            pos = if (remote != null) remote.posMs + if (remote.playing) SystemClock.elapsedRealtime() - remote.posAt else 0 else app.engine.position()
            delay(500)
        }
    }
    val send: (String, Any?) -> Unit = { op, v -> if (remote != null) app.link.command(remote.dev, op, v) }
    // the lines of the song: the one before, the one that sounds now and the next one
    var lyrics by remember(t.id) { mutableStateOf<Lyrics?>(null) }
    LaunchedEffect(t.id) { lyrics = LyricsApi.fromComputer(app, t) ?: withContext(Dispatchers.IO) { runCatching { LyricsApi.fetch(t) }.getOrNull() } }
    val liked by app.liked.collectAsState()
    val sleep by app.sleepMin.collectAsState()
    val scope = rememberCoroutineScope()
    /** the queue shown: its tracks, the one that plays, and (of the computer's queue) the place of each in its order */
    var queue by remember { mutableStateOf<Triple<List<Track>, Int, List<Int>?>?>(null) }
    var menu by remember { mutableStateOf(false) }
    var pickList by remember { mutableStateOf(false) }
    var pickShared by remember { mutableStateOf(false) }
    var saveTick by remember { mutableIntStateOf(0) }
    val ctx = androidx.compose.ui.platform.LocalContext.current
    LaunchedEffect(Unit) { if (app.liked.value.isEmpty() && app.store.linked) runCatching { val a = app.link.rpc("liked").getJSONArray("data"); app.liked.value = (0 until a.length()).map { a.getLong(it) }.toSet() } }
    val isLiked = t.id in liked
    val room by app.rooms.state.collectAsState()
    val inRoom = room != null && app.rooms.following.value

    Box(Modifier.fillMaxSize().background(C.bg)) {
        Column(Modifier.fillMaxSize().statusBarsPadding().navigationBarsPadding().padding(horizontal = 24.dp)) {
            Row(Modifier.fillMaxWidth().padding(vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.size(44.dp).pressable(onClose), contentAlignment = Alignment.CenterStart) { Icon(Ico.Down, 28.dp) }
                if (inRoom) Row(Modifier.weight(1f), horizontalArrangement = Arrangement.Center) {
                    Row(Modifier.clip(RoundedCornerShape(14.dp)).background(C.warm).border(1.dp, C.accent, RoundedCornerShape(14.dp)).pressable(onRooms).padding(horizontal = 12.dp, vertical = 7.dp), verticalAlignment = Alignment.CenterVertically) {
                        SIcon("rooms", C.accent)
                        Text("${room!!.name} · ${room!!.members.size}", color = C.accent, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(start = 6.dp), maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                } else Text(
                    if (remote != null) L.t("Играет на {0}").replace("{0}", L.t(remote.name.ifEmpty { "компьютере" })) else "Играет на этом телефоне",
                    color = C.muted, fontSize = 13.sp, modifier = Modifier.weight(1f).pressable(onDevices), maxLines = 1, textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                )
                Box(Modifier.size(44.dp), contentAlignment = Alignment.CenterEnd) {
                    Box(Modifier.size(44.dp).pressable { menu = true }, contentAlignment = Alignment.CenterEnd) { SIcon("dots", if (remote != null) C.accent else C.text) }
                    androidx.compose.material3.DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                        val have = Downloads.localFile(t.id) != null
                        val busy = Downloads.progressOf(t.id) != null
                        androidx.compose.material3.DropdownMenuItem(
                            text = { Text(if (have) "Удалить из скачанного" else if (busy) "Скачивается…" else "Скачать для офлайна", color = C.text) },
                            onClick = {
                                menu = false
                                if (have) { Downloads.remove(app, t.id); saveTick++ }
                                else if (!busy) scope.launch {
                                    val msg = withContext(Dispatchers.IO) { try { Downloads.save(ctx, app, t); "Скачано: ${t.title}" } catch (e: Exception) { e.message ?: "Не вышло скачать" } }
                                    android.widget.Toast.makeText(ctx, msg, android.widget.Toast.LENGTH_LONG).show()
                                    saveTick++
                                }
                            },
                        )
                        // the queue: this track next, or at the end (on the computer when it plays there)
                        val toQueue = { next: Boolean ->
                            menu = false
                            if (remote != null) scope.launch { runCatching { app.link.rpc("enqueue", org.json.JSONObject().put("track", t.id).put("next", next)) } }
                            else if (next) app.engine.insertNext(t) else app.engine.append(listOf(t))
                            android.widget.Toast.makeText(ctx, if (next) "Сыграет следующим" else "Добавлено в конец очереди", android.widget.Toast.LENGTH_SHORT).show()
                        }
                        androidx.compose.material3.DropdownMenuItem(text = { Text("Играть следующим", color = C.text) }, onClick = { toQueue(true) })
                        androidx.compose.material3.DropdownMenuItem(text = { Text("В конец очереди", color = C.text) }, onClick = { toQueue(false) })
                        androidx.compose.material3.DropdownMenuItem(text = { Text("Добавить в плейлист…", color = C.text) }, onClick = { menu = false; pickList = true })
                        androidx.compose.material3.DropdownMenuItem(text = { Text("В общий плейлист…", color = C.text) }, onClick = { menu = false; pickShared = true })
                        androidx.compose.material3.DropdownMenuItem(text = { Text("Открыть на SoundCloud", color = C.text) }, onClick = {
                            menu = false
                            scope.launch {
                                val url = withContext(Dispatchers.IO) { app.api.permalink(t.id) }
                                if (url == null) android.widget.Toast.makeText(ctx, L.t("Не получилось"), android.widget.Toast.LENGTH_SHORT).show()
                                else ctx.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(url)).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK))
                            }
                        })
                        androidx.compose.material3.DropdownMenuItem(text = { Text("Искать текст заново", color = C.text) }, onClick = {
                            menu = false
                            scope.launch {
                                lyrics = null
                                lyrics = LyricsApi.fromComputer(app, t, force = true) ?: withContext(Dispatchers.IO) { runCatching { LyricsApi.fetch(t) }.getOrNull() }
                            }
                        })
                        // the speed of this phone's own playback: 0.75, 1, 1.25, 1.5
                        if (remote == null) androidx.compose.material3.DropdownMenuItem(text = { Text("Скорость: ×" + app.engine.speed.toString().removeSuffix(".0"), color = C.text) }, onClick = {
                            val steps = listOf(0.75f, 1f, 1.25f, 1.5f)
                            val next = steps[(steps.indexOf(app.engine.speed).let { if (it < 0) 1 else it } + 1) % steps.size]
                            app.engine.speed = next
                            app.engine.applySpeed(next)
                            menu = false
                        })
                        // a link to the track, or to the second that plays now, to send to somebody
                        val share = { withTime: Boolean ->
                            menu = false
                            scope.launch {
                                val url = withContext(Dispatchers.IO) { app.api.permalink(t.id) }
                                if (url == null) android.widget.Toast.makeText(ctx, L.t("Не получилось"), android.widget.Toast.LENGTH_SHORT).show()
                                else {
                                    val link = if (withTime) url + "#t=" + (if (remote != null) remote.posMs else app.engine.position()).div(1000) else url
                                    val send = android.content.Intent(android.content.Intent.ACTION_SEND).setType("text/plain").putExtra(android.content.Intent.EXTRA_TEXT, link)
                                    ctx.startActivity(android.content.Intent.createChooser(send, null).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK))
                                }
                            }
                        }
                        androidx.compose.material3.DropdownMenuItem(text = { Text("Поделиться ссылкой", color = C.text) }, onClick = { share(false) })
                        androidx.compose.material3.DropdownMenuItem(text = { Text("Ссылка с текущей секундой", color = C.text) }, onClick = { share(true) })
                        androidx.compose.material3.DropdownMenuItem(text = { Text("Играть на…", color = C.text) }, onClick = { menu = false; onDevices() })
                    }
                }
            }
            Cover(t.artwork?.replace("-t300x300", "-t500x500"), 320.dp, 28.dp, Modifier.fillMaxWidth().aspectRatio(1f), sharp = true)
            Spacer(Modifier.height(16.dp))
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(t.title, color = C.text, fontSize = 22.sp, fontWeight = FontWeight.Bold, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    Text(t.artist, color = C.muted, fontSize = 14.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                Box(Modifier.padding(start = 12.dp).size(48.dp).clip(RoundedCornerShape(16.dp)).background(C.panel).pressable {
                    val on = !isLiked
                    app.liked.value = if (on) app.liked.value + t.id else app.liked.value - t.id
                    scope.launch { runCatching { Writes.call(app, "like", org.json.JSONObject().put("track", t.id).put("on", on)) }.onFailure { app.liked.value = if (on) app.liked.value - t.id else app.liked.value + t.id } }
                }, contentAlignment = Alignment.Center) { Icon(if (isLiked) Ico.Heart else Ico.HeartOff, 24.dp, if (isLiked) C.accent else C.text) }
            }
            if (inRoom) room?.let { r ->
                if (r.line.isNotEmpty()) Text(r.line, color = C.muted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(top = 10.dp).fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(C.panel).padding(horizontal = 14.dp, vertical = 10.dp))
                Text("СЛУШАЮТ", color = C.faint, fontSize = 10.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(top = 12.dp, bottom = 6.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    r.members.take(4).forEach { m ->
                        Box(Modifier.size(34.dp).clip(androidx.compose.foundation.shape.CircleShape).background(C.panel2).border(if (m.you) 2.dp else 0.dp, C.accent, androidx.compose.foundation.shape.CircleShape), contentAlignment = Alignment.Center) { Text(m.name.take(1).uppercase(), color = C.text, fontSize = 13.sp, fontWeight = FontWeight.SemiBold) }
                    }
                    if (r.members.size > 4) Text("вы + ${r.members.size - 1}", color = C.muted, fontSize = 12.sp)
                }
            }
            // the song, three lines: it makes the middle of the screen
            Column(Modifier.weight(1f).fillMaxWidth().pressable(onLyrics), verticalArrangement = Arrangement.Center) {
                val ly = lyrics?.takeIf { it.synced }
                if (ly != null) {
                    val i = ly.lines.indexOfLast { it.atMs <= pos + 250 }
                    Text(ly.lines.getOrNull(i - 1)?.text ?: "", color = C.faint, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    val cur = ly.lines.getOrNull(i)
                    Text(
                        if (cur == null || cur.text.isEmpty()) androidx.compose.ui.text.AnnotatedString("♪") else LyricsApi.coloured(cur, pos, C.text, C.accent, C.faint),
                        color = C.text, fontSize = 20.sp, fontWeight = FontWeight.Bold, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(vertical = 6.dp),
                    )
                    Text(ly.lines.getOrNull(i + 1)?.text ?: "", color = C.faint, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                } else if (lyrics != null) {
                    Text(lyrics!!.lines.firstOrNull { it.text.isNotBlank() }?.text ?: "", color = C.faint, fontSize = 14.sp, maxLines = 2, overflow = TextOverflow.Ellipsis)
                }
            }
            Slider(
                value = (dragging ?: if (dur > 0) pos.toFloat() / dur else 0f).coerceIn(0f, 1f),
                onValueChange = { dragging = it },
                onValueChangeFinished = {
                    dragging?.let { f ->
                        val ms = (f * dur).toLong()
                        if (remote != null) send("seek", ms) else app.engine.seek(ms)
                        pos = ms
                    }
                    dragging = null
                },
                colors = SliderDefaults.colors(thumbColor = C.text, activeTrackColor = C.accent, inactiveTrackColor = C.panel2),
            )
            Row(Modifier.fillMaxWidth()) {
                Text(fmt(dragging?.let { (it * dur).toLong() } ?: pos), color = C.muted, fontSize = 12.sp, modifier = Modifier.weight(1f))
                Text(fmt(dur), color = C.muted, fontSize = 12.sp)
            }
            Row(Modifier.fillMaxWidth().padding(vertical = 14.dp), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.size(44.dp).pressable { if (remote == null) app.engine.toggleShuffle() }, contentAlignment = Alignment.Center) { SIcon("shuffle", if (state.shuffle && remote == null) C.accent else C.muted) }
                Box(Modifier.size(52.dp).pressable { if (remote != null) send("prev", null) else app.engine.prev() }, contentAlignment = Alignment.Center) { Icon(Ico.Prev, 32.dp) }
                Box(Modifier.size(72.dp).clip(if (C.squareButton) RoundedCornerShape(26.dp) else androidx.compose.foundation.shape.CircleShape).background(C.text).pressable { if (remote != null) send(if (shown.playing) "pause" else "play", null) else app.engine.toggle() }, contentAlignment = Alignment.Center) {
                    Icon(if (shown.playing) Ico.Pause else Ico.Play, 40.dp, C.bg)
                }
                Box(Modifier.size(52.dp).pressable {
                    if (inRoom) scope.launch { runCatching { app.link.rpc(if (room?.voteSkip == true) "room_vote" else "room_next") } }
                    else if (remote != null) send("next", null) else app.engine.next()
                }, contentAlignment = Alignment.Center) { Icon(Ico.Next, 32.dp) }
                if (inRoom && room?.voteSkip == true) Box(Modifier.clip(RoundedCornerShape(14.dp)).background(C.warm).pressable { scope.launch { runCatching { app.link.rpc("room_vote") } } }.padding(horizontal = 12.dp, vertical = 10.dp), contentAlignment = Alignment.Center) {
                    Text("${room!!.skipVotes}/${room!!.skipNeed}", color = C.accent, fontSize = 14.sp, fontWeight = FontWeight.Bold)
                } else Box(Modifier.size(44.dp).pressable { if (remote == null) app.engine.cycleRepeat() }, contentAlignment = Alignment.Center) {
                    SIcon("repeat", if (state.repeat > 0 && remote == null) C.accent else C.muted)
                    if (state.repeat == 2 && remote == null) Text("1", color = C.accent, fontSize = 9.sp, fontWeight = FontWeight.Bold)
                }
            }
            // the equalizer, the rooms, the song (with its words), the sleep timer, the queue
            Row(Modifier.fillMaxWidth().padding(bottom = 10.dp).clip(RoundedCornerShape(24.dp)).background(C.panel).padding(vertical = 6.dp), horizontalArrangement = Arrangement.SpaceEvenly) {
                Box(Modifier.size(46.dp).pressable(onEq), contentAlignment = Alignment.Center) { SIcon("eq", C.text) }
                Box(Modifier.size(46.dp).pressable(onRooms), contentAlignment = Alignment.Center) { SIcon("rooms", C.text) }
                Box(Modifier.size(46.dp).pressable(onLyrics), contentAlignment = Alignment.Center) { SIcon("translate", C.text) }
                Box(Modifier.size(46.dp).pressable { app.setSleep(when (sleep) { 0 -> 15; 15 -> 30; 30 -> 60; else -> 0 }) }, contentAlignment = Alignment.Center) {
                    SIcon("moon", if (sleep > 0) C.accent else C.text)
                    if (sleep > 0) Text("$sleep", color = C.accent, fontSize = 9.sp, fontWeight = FontWeight.Bold, modifier = Modifier.align(Alignment.BottomEnd).padding(4.dp))
                }
                Box(Modifier.size(46.dp).pressable {
                    if (remote == null) app.engine.queue { l, i -> queue = Triple(l, i, null) }
                    // the music plays on the computer: its queue is what the computer has
                    else scope.launch {
                        runCatching {
                            val d = app.link.rpc("queue").getJSONObject("data")
                            val a = d.getJSONArray("items")
                            val tracks = ArrayList<Track>(); val ats = ArrayList<Int>()
                            for (k in 0 until a.length()) {
                                val o = a.getJSONObject(k); val j = o.getJSONObject("t")
                                tracks.add(Track(j.getLong("id"), j.optString("title"), j.optString("artist"), j.optLong("dur"), j.optString("art").ifEmpty { null }))
                                ats.add(o.getInt("at"))
                            }
                            val cur = if (d.isNull("current")) -1 else ats.indexOf(d.getInt("current"))
                            queue = Triple(tracks, cur, ats)
                        }.onFailure { android.widget.Toast.makeText(ctx, "Очередь компьютера не пришла: ${it.message}", android.widget.Toast.LENGTH_SHORT).show() }
                    }
                }, contentAlignment = Alignment.Center) { SIcon("queue", C.text) }
            }
        }
        if (pickList) PlaylistPicker(app, t) { pickList = false }
        if (pickShared) PlaylistPicker(app, t, shared = true) { pickShared = false }
        queue?.let { (list, current, ats) ->
            Box(Modifier.fillMaxSize().background(androidx.compose.ui.graphics.Color(0xAA000000)).pressable { queue = null }) {
                Column(Modifier.align(Alignment.BottomCenter).fillMaxWidth().height(480.dp).clip(RoundedCornerShape(topStart = 24.dp, topEnd = 24.dp)).background(C.panel).consumeTaps().navigationBarsPadding().padding(top = 16.dp)) {
                    Text(if (ats != null) "Очередь на компьютере" else "Очередь", color = C.text, fontSize = 18.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(start = 20.dp, bottom = 8.dp))
                    if (list.isEmpty()) Text("В очереди пусто", color = C.muted, fontSize = 14.sp, modifier = Modifier.padding(start = 20.dp))
                    val st = androidx.compose.foundation.lazy.rememberLazyListState(maxOf(current - 1, 0))
                    androidx.compose.foundation.lazy.LazyColumn(Modifier.fillMaxSize(), state = st) {
                        itemsIndexed(list, key = { i, tr -> "$i-${tr.id}" }) { i, tr ->
                            TrackRow(tr, i == current) {
                                if (ats != null && remote != null) send("qplay", ats[i]) else app.engine.jump(i)
                                queue = null
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
internal fun DevicesPanel(app: App, shown: Shown, onClose: () -> Unit, onLinkPc: () -> Unit, onEq: () -> Unit, onControl: (String) -> Unit) {
    val peers by app.link.peers.collectAsState()
    val state by app.engine.state.collectAsState()
    val now by produceState(SystemClock.elapsedRealtime()) { while (true) { delay(3000); value = SystemClock.elapsedRealtime() } }
    val remote = shown.remote
    Box(Modifier.fillMaxSize().background(androidx.compose.ui.graphics.Color(0xAA000000)).pressable(onClose)) {
        Column(
            Modifier.align(Alignment.BottomCenter).fillMaxWidth().clip(RoundedCornerShape(topStart = 24.dp, topEnd = 24.dp)).background(C.panel)
                .consumeTaps().navigationBarsPadding().padding(20.dp),
        ) {
            Text("Управлять", color = C.text, fontSize = 20.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(bottom = 14.dp))
            DeviceRow("Этот телефон", app.store.deviceName, Ico.Phone, remote == null, true) {
                onControl("")
                onClose()
            }
            peers.values.sortedBy { it.name }.forEach { p ->
                val online = now - p.seenAt < 100_000
                val sub = when {
                    !online -> "не в сети"
                    p.playing && p.track != null -> "играет: ${p.track.title}"
                    else -> "в сети"
                }
                DeviceRow(p.name.ifEmpty { "Компьютер" }, sub, if (p.kind == "phone") Ico.Phone else Ico.Monitor, remote?.dev == p.dev, online) {
                    onControl(p.dev)
                    onClose()
                }
                if (online) Row(Modifier.padding(start = 6.dp, bottom = 10.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    if (state.track != null) Text("Перенести туда музыку", color = C.accent, fontSize = 13.sp, modifier = Modifier.pressable { app.link.pushTo(p.dev); onControl(p.dev); onClose() }.padding(vertical = 4.dp))
                    if (p.track != null) Text("Забрать сюда", color = C.accent, fontSize = 13.sp, modifier = Modifier.pressable { app.link.pullFrom(p.dev); onControl(""); onClose() }.padding(vertical = 4.dp))
                }
            }
            if (!app.store.linked) {
                Text("Подключить компьютер", color = C.bg, fontSize = 15.sp, fontWeight = FontWeight.SemiBold,
                    modifier = Modifier.fillMaxWidth().padding(top = 4.dp).clip(RoundedCornerShape(14.dp)).background(C.text).pressable(onLinkPc).padding(vertical = 14.dp),
                    textAlign = androidx.compose.ui.text.style.TextAlign.Center)
                Text("QR-код: на компьютере Настройки, Интеграции, «Подключить телефон».", color = C.muted, fontSize = 12.sp, modifier = Modifier.padding(top = 6.dp))
            } else if (peers.isEmpty()) Text("Компьютер появится здесь, когда в нём запущен SC Desk.", color = C.muted, fontSize = 13.sp, modifier = Modifier.padding(vertical = 6.dp))
            Spacer(Modifier.height(10.dp))
            Row(
                Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(C.panel2).pressable(onEq).padding(horizontal = 16.dp, vertical = 14.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text("Эквалайзер", color = C.text, fontSize = 15.sp, fontWeight = FontWeight.Medium, modifier = Modifier.weight(1f))
                Text(if (app.eq.state.enabled) "включён" else "выключен", color = C.muted, fontSize = 13.sp)
            }
            Spacer(Modifier.height(10.dp))
            var vol by remember(remote?.dev) { mutableStateOf(remote?.volume ?: state.volume) }
            Text(L.t("ГРОМКОСТЬ · {0}").replace("{0}", if (remote != null) remote.name.uppercase().ifEmpty { L.t("КОМПЬЮТЕР") } else L.t("ЭТОТ ТЕЛЕФОН")), color = C.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
            Slider(
                value = vol, onValueChange = { vol = it; if (remote != null) app.link.command(remote.dev, "vol", it.toDouble()) else app.engine.volume(it) },
                colors = SliderDefaults.colors(thumbColor = C.text, activeTrackColor = C.accent, inactiveTrackColor = C.panel2),
            )
        }
    }
}

@Composable
private fun DeviceRow(name: String, sub: String, icon: Ico, active: Boolean, enabled: Boolean, onClick: () -> Unit) {
    val shape = RoundedCornerShape(14.dp)
    Row(
        Modifier.fillMaxWidth().padding(bottom = 8.dp).clip(shape).background(if (active) C.warm else C.bg)
            .border(1.dp, if (active) C.accent else C.border, shape).pressable { if (enabled) onClick() }.padding(14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, 26.dp, if (active) C.accent else if (enabled) C.text else C.faint)
        Column(Modifier.padding(start = 14.dp).weight(1f)) {
            Text(name, color = if (enabled) C.text else C.faint, fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
            Text(sub, color = if (active) C.accent else if (enabled) C.muted else C.faint, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        if (active) Icon(Ico.Check, 22.dp, C.accent)
    }
}
