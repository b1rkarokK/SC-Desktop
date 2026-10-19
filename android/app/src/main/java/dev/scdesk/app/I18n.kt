package dev.scdesk.app

import androidx.compose.foundation.text.InlineTextContent
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.TextUnit

/**
 * The language of the app: «ru» (the texts are written in Russian) or «en» (each text found in [STRINGS_EN] is
 * replaced as it is drawn; a text that is not there stays as it is). A text with a number or a name in it is
 * kept with «{0}», «{1}» in its place, so «Скачивается {0} из {1}» also fits «Скачивается 3 из 10».
 */
object L {
    var lang by mutableStateOf("ru")

    private val plain: Map<String, String> by lazy { STRINGS_EN.filterKeys { !it.contains("{0}") } }
    private val patterns: List<Pair<Regex, String>> by lazy {
        STRINGS_EN.filterKeys { it.contains("{0}") }.map { (k, v) ->
            // «{0}» in the Russian text matches any piece; the English text has the pieces put back by number
            val rx = Regex("^" + k.split(Regex("\\{\\d+}")).joinToString("(.+?)") { Regex.escape(it) } + "$", RegexOption.DOT_MATCHES_ALL)
            rx to v
        }.sortedByDescending { it.first.pattern.length }
    }

    private val seen = HashMap<String, String>()

    fun t(s: String): String {
        if (lang != "en" || s.isEmpty()) return s
        plain[s]?.let { return it }
        // a long text (the notes of a version, a line of lyrics) is never one of the short texts with a hole in it
        if (s.length > 160) return s
        seen[s]?.let { return it }
        val out = try {
            var res = s
            for ((rx, v) in patterns) {
                val m = rx.find(s) ?: continue
                var o = v
                m.groupValues.drop(1).forEachIndexed { i, g -> o = o.replace("{$i}", g) }
                res = o
                break
            }
            res
        } catch (e: Throwable) {
            s
        }
        if (seen.size > 600) seen.clear()
        seen[s] = out
        return out
    }

    /** The language the phone is set to, as the first choice. */
    fun systemDefault(): String = if (java.util.Locale.getDefault().language == "ru") "ru" else "en"
}

/** The text of the app: the same as the material one, with the words put into the chosen language. */
@Composable
fun Text(
    text: String,
    modifier: Modifier = Modifier,
    color: Color = Color.Unspecified,
    fontSize: TextUnit = TextUnit.Unspecified,
    fontStyle: FontStyle? = null,
    fontWeight: FontWeight? = null,
    fontFamily: FontFamily? = null,
    letterSpacing: TextUnit = TextUnit.Unspecified,
    textDecoration: TextDecoration? = null,
    textAlign: TextAlign? = null,
    lineHeight: TextUnit = TextUnit.Unspecified,
    overflow: TextOverflow = TextOverflow.Clip,
    softWrap: Boolean = true,
    maxLines: Int = Int.MAX_VALUE,
    minLines: Int = 1,
    onTextLayout: ((TextLayoutResult) -> Unit)? = null,
    style: TextStyle = androidx.compose.material3.LocalTextStyle.current,
) = androidx.compose.material3.Text(
    L.t(text), modifier, color, fontSize, fontStyle, fontWeight, fontFamily, letterSpacing, textDecoration, textAlign, lineHeight, overflow, softWrap, maxLines, minLines, onTextLayout, style,
)

@Composable
fun Text(
    text: AnnotatedString,
    modifier: Modifier = Modifier,
    color: Color = Color.Unspecified,
    fontSize: TextUnit = TextUnit.Unspecified,
    fontStyle: FontStyle? = null,
    fontWeight: FontWeight? = null,
    fontFamily: FontFamily? = null,
    letterSpacing: TextUnit = TextUnit.Unspecified,
    textDecoration: TextDecoration? = null,
    textAlign: TextAlign? = null,
    lineHeight: TextUnit = TextUnit.Unspecified,
    overflow: TextOverflow = TextOverflow.Clip,
    softWrap: Boolean = true,
    maxLines: Int = Int.MAX_VALUE,
    minLines: Int = 1,
    inlineContent: Map<String, InlineTextContent> = mapOf(),
    onTextLayout: (TextLayoutResult) -> Unit = {},
    style: TextStyle = androidx.compose.material3.LocalTextStyle.current,
) = androidx.compose.material3.Text(
    text, modifier, color, fontSize, fontStyle, fontWeight, fontFamily, letterSpacing, textDecoration, textAlign, lineHeight, overflow, softWrap, maxLines, minLines, inlineContent, onTextLayout, style,
)
