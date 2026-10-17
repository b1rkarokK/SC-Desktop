package dev.scdesk.app

import android.graphics.BitmapFactory
import android.util.LruCache
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.Request

/** The palettes: «Тёплая» (the design of Penpot, page «андроид»), OLED (true black) and light. */
object C {
    private class P(val bg: Color, val panel: Color, val panel2: Color, val border: Color, val text: Color, val muted: Color, val faint: Color, val accent: Color, val warm: Color)

    private val sets = listOf(
        P(Color(0xFF161414), Color(0xFF221F1F), Color(0xFF2C2828), Color(0xFF2E2A2A), Color(0xFFF2EBE3), Color(0xFFA0968D), Color(0xFF5E5650), Color(0xFFFF7B3F), Color(0xFF2E1D14)),
        P(Color(0xFF000000), Color(0xFF0E0D0D), Color(0xFF1A1818), Color(0xFF1F1C1C), Color(0xFFF2EBE3), Color(0xFFA0968D), Color(0xFF5E5650), Color(0xFFFF7B3F), Color(0xFF1A100A)),
        P(Color(0xFFFAF6F1), Color(0xFFFFFFFF), Color(0xFFEFE8E0), Color(0xFFE4DAD0), Color(0xFF241E1A), Color(0xFF6F655C), Color(0xFFB5A99D), Color(0xFFE5652A), Color(0xFFFCE9DD)),
    )

    /** 0 «Тёплая», 1 OLED, 2 светлая; a state, so that the whole screen is drawn again when it changes */
    var mode by androidx.compose.runtime.mutableIntStateOf(0)
    var followSystem by androidx.compose.runtime.mutableStateOf(false)
    var systemDark by androidx.compose.runtime.mutableStateOf(true)
    /** the swatch of the accent (0: the one of the theme), and the one taken from the cover of the playing track */
    var accentIdx by androidx.compose.runtime.mutableIntStateOf(0)
    var useCover by androidx.compose.runtime.mutableStateOf(false)
    var coverAccent by androidx.compose.runtime.mutableStateOf<Color?>(null)
    /** the density of the lists: 0 tight, 1 usual, 2 roomy */
    var density by androidx.compose.runtime.mutableIntStateOf(1)
    var squareButton by androidx.compose.runtime.mutableStateOf(true)
    var anim by androidx.compose.runtime.mutableStateOf(true)

    private val swatches = listOf(0xFFFF7B3F, 0xFFEC5B7A, 0xFFB77DEE, 0xFF5B9DF5, 0xFF3FBF9A, 0xFFF5C242, 0xFFE8E8E8)

    /** the theme that is drawn: the chosen one, or by the system when it is asked for (dark ones keep the chosen dark) */
    val effective: Int get() = if (followSystem) (if (systemDark) (if (mode == 2) 0 else mode) else 2) else mode
    private val p get() = sets[effective.coerceIn(0, 2)]
    val bg get() = p.bg
    val panel get() = p.panel
    val panel2 get() = p.panel2
    val border get() = p.border
    val text get() = p.text
    val muted get() = p.muted
    val faint get() = p.faint
    val accent: Color get() = (if (useCover) coverAccent else null) ?: if (accentIdx in 1 until swatches.size) Color(swatches[accentIdx]) else p.accent
    val warm get() = p.warm

    /** for the little pictures of the themes: background, panel, text, accent */
    fun preview(i: Int): List<Color> = sets[i.coerceIn(0, 2)].let { listOf(it.bg, it.panel, it.text, it.accent) }
}

@Composable
fun SCTheme(content: @Composable () -> Unit) {
    C.systemDark = androidx.compose.foundation.isSystemInDarkTheme()
    val light = C.effective == 2
    val view = androidx.compose.ui.platform.LocalView.current
    androidx.compose.runtime.SideEffect {
        // the clock and the battery in the status bar must stay readable
        (view.context as? android.app.Activity)?.window?.let { androidx.core.view.WindowCompat.getInsetsController(it, view).apply { isAppearanceLightStatusBars = light; isAppearanceLightNavigationBars = light } }
    }
    MaterialTheme(
        colorScheme = if (light) androidx.compose.material3.lightColorScheme(background = C.bg, surface = C.panel, onSurface = C.text, primary = C.accent, onPrimary = C.bg, onBackground = C.text)
        else darkColorScheme(background = C.bg, surface = C.panel, onSurface = C.text, primary = C.accent, onPrimary = C.bg, onBackground = C.text),
        content = content,
    )
}

/**
 * A press makes the button a little smaller and darker and lets it go back at once, with a spring: the same on
 * every button of the player and of the lists.
 */
fun Modifier.pressable(onClick: () -> Unit): Modifier = if (!C.anim) this.composed { clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onClick) } else composed {
    val src = remember { MutableInteractionSource() }
    val pressed by src.collectIsPressedAsState()
    val scale by animateFloatAsState(if (pressed) 0.9f else 1f, spring(dampingRatio = 0.55f, stiffness = Spring.StiffnessMedium), label = "press")
    val alpha by animateFloatAsState(if (pressed) 0.75f else 1f, spring(stiffness = Spring.StiffnessMedium), label = "pressAlpha")
    this
        .graphicsLayer { scaleX = scale; scaleY = scale; this.alpha = alpha }
        .clickable(interactionSource = src, indication = null, onClick = onClick)
}

enum class Ico { Play, Pause, Next, Prev, Heart, HeartOff, Back, Close, Device, Phone, Monitor, Check, Down, Search, Menu, Home, Wave }

/** The icons are drawn here (a few strokes each): no icon library in the package. */
@Composable
fun Icon(kind: Ico, size: Dp = 24.dp, color: Color = C.text) {
    Canvas(Modifier.size(size)) {
        val w = this.size.width
        val s = Stroke(width = w * 0.085f, cap = StrokeCap.Round, join = androidx.compose.ui.graphics.StrokeJoin.Round)
        fun p(x: Float, y: Float) = Offset(x * w, y * w)
        when (kind) {
            Ico.Play -> drawPath(Path().apply { moveTo(w * .3f, w * .2f); lineTo(w * .8f, w * .5f); lineTo(w * .3f, w * .8f); close() }, color)
            Ico.Pause -> {
                drawRoundRect(color, p(.26f, .2f), Size(w * .16f, w * .6f), androidx.compose.ui.geometry.CornerRadius(w * .04f))
                drawRoundRect(color, p(.58f, .2f), Size(w * .16f, w * .6f), androidx.compose.ui.geometry.CornerRadius(w * .04f))
            }
            Ico.Next -> {
                drawPath(Path().apply { moveTo(w * .22f, w * .22f); lineTo(w * .68f, w * .5f); lineTo(w * .22f, w * .78f); close() }, color)
                drawLine(color, p(.8f, .22f), p(.8f, .78f), w * .09f, StrokeCap.Round)
            }
            Ico.Prev -> {
                drawPath(Path().apply { moveTo(w * .78f, w * .22f); lineTo(w * .32f, w * .5f); lineTo(w * .78f, w * .78f); close() }, color)
                drawLine(color, p(.2f, .22f), p(.2f, .78f), w * .09f, StrokeCap.Round)
            }
            Ico.Heart, Ico.HeartOff -> {
                val path = Path().apply {
                    moveTo(w * .5f, w * .84f)
                    cubicTo(w * .1f, w * .56f, w * .1f, w * .22f, w * .32f, w * .22f)
                    cubicTo(w * .42f, w * .22f, w * .48f, w * .28f, w * .5f, w * .34f)
                    cubicTo(w * .52f, w * .28f, w * .58f, w * .22f, w * .68f, w * .22f)
                    cubicTo(w * .9f, w * .22f, w * .9f, w * .56f, w * .5f, w * .84f)
                    close()
                }
                if (kind == Ico.Heart) drawPath(path, color) else drawPath(path, color, style = s)
            }
            Ico.Home -> {
                drawPath(Path().apply { moveTo(w * .14f, w * .48f); lineTo(w * .5f, w * .16f); lineTo(w * .86f, w * .48f) }, color, style = s)
                drawPath(Path().apply { moveTo(w * .24f, w * .42f); lineTo(w * .24f, w * .82f); lineTo(w * .76f, w * .82f); lineTo(w * .76f, w * .42f) }, color, style = s)
            }
            Ico.Wave -> {
                listOf(.18f to .22f, .34f to .5f, .5f to .74f, .66f to .38f, .82f to .2f).forEach { (x, h) ->
                    drawLine(color, p(x, .5f - h / 2), p(x, .5f + h / 2), s.width, StrokeCap.Round)
                }
            }
            Ico.Search -> {
                drawCircle(color, w * .28f, p(.45f, .45f), style = s)
                drawLine(color, p(.66f, .66f), p(.84f, .84f), s.width, StrokeCap.Round)
            }
            Ico.Menu -> {
                drawLine(color, p(.2f, .3f), p(.8f, .3f), s.width, StrokeCap.Round)
                drawLine(color, p(.2f, .5f), p(.8f, .5f), s.width, StrokeCap.Round)
                drawLine(color, p(.2f, .7f), p(.8f, .7f), s.width, StrokeCap.Round)
            }
            Ico.Back -> {
                drawLine(color, p(.8f, .5f), p(.2f, .5f), s.width, StrokeCap.Round)
                drawLine(color, p(.2f, .5f), p(.45f, .25f), s.width, StrokeCap.Round)
                drawLine(color, p(.2f, .5f), p(.45f, .75f), s.width, StrokeCap.Round)
            }
            Ico.Close -> {
                drawLine(color, p(.25f, .25f), p(.75f, .75f), s.width, StrokeCap.Round)
                drawLine(color, p(.75f, .25f), p(.25f, .75f), s.width, StrokeCap.Round)
            }
            Ico.Down -> {
                drawLine(color, p(.25f, .38f), p(.5f, .63f), s.width, StrokeCap.Round)
                drawLine(color, p(.5f, .63f), p(.75f, .38f), s.width, StrokeCap.Round)
            }
            Ico.Check -> {
                drawLine(color, p(.22f, .52f), p(.42f, .72f), s.width, StrokeCap.Round)
                drawLine(color, p(.42f, .72f), p(.78f, .3f), s.width, StrokeCap.Round)
            }
            Ico.Phone -> {
                drawRoundRect(color, p(.3f, .12f), Size(w * .4f, w * .76f), androidx.compose.ui.geometry.CornerRadius(w * .08f), style = s)
                drawLine(color, p(.44f, .78f), p(.56f, .78f), s.width, StrokeCap.Round)
            }
            Ico.Monitor -> {
                drawRoundRect(color, p(.12f, .2f), Size(w * .76f, w * .5f), androidx.compose.ui.geometry.CornerRadius(w * .06f), style = s)
                drawLine(color, p(.4f, .84f), p(.6f, .84f), s.width, StrokeCap.Round)
                drawLine(color, p(.5f, .7f), p(.5f, .84f), s.width, StrokeCap.Round)
            }
            Ico.Device -> {
                drawRoundRect(color, p(.08f, .22f), Size(w * .6f, w * .42f), androidx.compose.ui.geometry.CornerRadius(w * .05f), style = s)
                drawRoundRect(color, p(.66f, .4f), Size(w * .26f, w * .46f), androidx.compose.ui.geometry.CornerRadius(w * .05f), style = s)
                drawLine(color, p(.24f, .8f), p(.5f, .8f), s.width, StrokeCap.Round)
            }
        }
    }
}

// ---------------------------------------------------------------- covers

internal object CoverCache {
    /** The colour of the cover that suits an accent: the most vivid of its pixels, made bright enough for the dark theme (blocking). */
    fun accentOf(url: String): Color? {
        val bmp = (map.get(url) ?: fromDisk(url)?.let { decode(it) })?.asAndroidBitmap() ?: return null
        val small = android.graphics.Bitmap.createScaledBitmap(bmp, 24, 24, true)
        var best = 0f
        var pick: Int? = null
        val hsv = FloatArray(3)
        for (x in 0 until 24) for (y in 0 until 24) {
            val px = small.getPixel(x, y)
            android.graphics.Color.colorToHSV(px, hsv)
            val score = hsv[1] * hsv[2]
            if (hsv[2] > .3f && score > best) { best = score; pick = px }
        }
        val c = pick ?: return null
        android.graphics.Color.colorToHSV(c, hsv)
        hsv[1] = hsv[1].coerceIn(.45f, 1f)
        hsv[2] = hsv[2].coerceIn(.75f, 1f)
        return Color(android.graphics.Color.HSVToColor(hsv))
    }

    /** the picture mirror that answered last: the others are tried only when it stops */
    @Volatile var mirror = "i1"

    /** the covers kept on the disk between runs (set by the app) */
    @Volatile var dir: java.io.File? = null

    /** its own queue: many covers at once must not hold up the lists and the link */
    val client: okhttp3.OkHttpClient by lazy {
        http.newBuilder()
            .dispatcher(okhttp3.Dispatcher().apply { maxRequests = 24; maxRequestsPerHost = 8 })
            .callTimeout(12, java.util.concurrent.TimeUnit.SECONDS).build()
    }

    /** the picture is the same on i1..i4: one file for all */
    private fun file(url: String): java.io.File? {
        val d = dir ?: return null
        val norm = url.replace(Regex("://i[1-4][.]sndcdn[.]com/"), "://i1.sndcdn.com/")
        val h = java.security.MessageDigest.getInstance("SHA-1").digest(norm.toByteArray()).joinToString("") { "%02x".format(it) }
        return java.io.File(d, h)
    }

    fun fromDisk(url: String): ByteArray? = file(url)?.takeIf { it.exists() }?.let { runCatching { it.readBytes() }.getOrNull() }

    fun toDisk(url: String, bytes: ByteArray) {
        val f = file(url) ?: return
        runCatching {
            f.parentFile?.mkdirs()
            f.writeBytes(bytes)
            val all = f.parentFile?.listFiles()
            if (all != null && all.size > 700) all.sortedBy { it.lastModified() }.take(all.size - 600).forEach { it.delete() }
        }
    }

    fun decode(bytes: ByteArray, sharp: Boolean = false): ImageBitmap? =
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = if (sharp) 1 else 2 })?.asImageBitmap()

    /** about 8 MB of decoded covers */
    val map = object : LruCache<String, ImageBitmap>(8 * 1024 * 1024) {
        override fun sizeOf(key: String, value: ImageBitmap) = value.width * value.height * 4
    }
}

/** A cover from its address (decoded small, kept in a small cache); a dark square while it loads. */
@Composable
/** [sharp]: the big player: decoded whole, not halved. */
fun Cover(url: String?, size: Dp, radius: Dp = 8.dp, modifier: Modifier = Modifier, sharp: Boolean = false) {
    var bmp by remember(url) { mutableStateOf(url?.let { CoverCache.map.get(it) }) }
    LaunchedEffect(url) {
        if (url == null || bmp != null) return@LaunchedEffect
        bmp = withContext(Dispatchers.IO) {
            CoverCache.fromDisk(url)?.let { b -> CoverCache.decode(b, sharp)?.let { CoverCache.map.put(url, it); return@withContext it } }
            // the picture is on i1..i4 alike: when one of them does not answer (a provider closes some addresses), the next
            val hosts = if (url.contains("://i1.sndcdn.com/")) (listOf(CoverCache.mirror) + listOf("i1", "i2", "i3", "i4")).distinct() else listOf(null)
            for (h in hosts) {
                val u = if (h == null) url else url.replace("://i1.sndcdn.com/", "://$h.sndcdn.com/")
                try {
                    val got = CoverCache.client.newCall(Request.Builder().url(u).header("User-Agent", USER_AGENT).build()).execute().use { r ->
                        if (!r.isSuccessful) return@use null
                        val bytes = r.body?.bytes() ?: return@use null
                        CoverCache.toDisk(url, bytes)
                        CoverCache.decode(bytes, sharp)
                    }
                    if (got != null) {
                        if (h != null) CoverCache.mirror = h
                        CoverCache.map.put(url, got)
                        return@withContext got
                    }
                } catch (e: Exception) {
                    android.util.Log.w("SCDesk", "cover ${e.javaClass.simpleName} $u")
                }
            }
            null
        }
    }
    Box(modifier.size(size).clip(RoundedCornerShape(radius)).background(C.panel2)) {
        bmp?.let { Image(it, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop) }
    }
}
