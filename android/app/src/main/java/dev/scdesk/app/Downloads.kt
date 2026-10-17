package dev.scdesk.app

import android.content.Context
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.Request
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.io.IOException
import java.util.concurrent.ConcurrentHashMap

/** A track kept on the phone for the offline: its file and what to show about it. */
data class Saved(val track: Track, val file: String, val size: Long, val at: Long)

/**
 * «Скачанное»: a track is saved the way the computer does it (src-tauri/src/downloads.rs): the pieces of its HLS stream
 * one after another into one file, which plays as it is. A track that is saved plays from the file: no net, no stream
 * address. The files and the list live in the app's own folder; the limit is the user's.
 */
object Downloads {
    private val saved = ConcurrentHashMap<Long, Saved>()
    private val running = ConcurrentHashMap<Long, Float>()
    @Volatile private var loaded = false

    private fun dir(ctx: Context) = File(ctx.filesDir, "dl").also { it.mkdirs() }

    /** reads the list (once) and forgets the files that are gone */
    @Synchronized
    fun load(ctx: Context, store: Store) {
        if (loaded) return
        loaded = true
        try {
            val a = JSONArray(store.downloads)
            for (i in 0 until a.length()) {
                val o = a.getJSONObject(i)
                val f = File(o.getString("f"))
                if (!f.isFile) continue
                val t = Track(o.getLong("i"), o.optString("t"), o.optString("a"), o.optLong("d"), o.optString("c").ifEmpty { null })
                saved[t.id] = Saved(t, f.absolutePath, f.length(), o.optLong("at"))
            }
        } catch (e: Exception) {
            // an unreadable list: nothing is kept
        }
        persist(store)
    }

    private fun persist(store: Store) {
        val a = JSONArray()
        saved.values.sortedBy { it.at }.forEach {
            a.put(JSONObject().put("i", it.track.id).put("t", it.track.title).put("a", it.track.artist).put("c", it.track.artwork ?: "").put("d", it.track.durationMs).put("f", it.file).put("at", it.at))
        }
        store.downloads = a.toString()
    }

    fun localFile(id: Long): File? = saved[id]?.let { File(it.file) }?.takeIf { it.isFile }
    fun all(): List<Saved> = saved.values.sortedByDescending { it.at }
    fun usedBytes(): Long = saved.values.sumOf { it.size }
    fun progressOf(id: Long): Float? = running[id]

    /** Whether this track is a mp3 (no init piece in its playlist) is told by the file's name. */
    fun mimeOf(f: File) = if (f.name.endsWith(".mp3")) androidx.media3.common.MimeTypes.AUDIO_MPEG else androidx.media3.common.MimeTypes.AUDIO_MP4

    /** Saves the track (blocking: the network). Fails with a short reason when the limit is reached. */
    fun save(ctx: Context, app: App, t: Track, progress: (Float) -> Unit = {}) {
        if (saved.containsKey(t.id) || running.containsKey(t.id)) return
        running[t.id] = 0f
        var out: File? = null
        try {
            val limit = app.store.downloadLimitGb * 1024L * 1024L * 1024L
            if (usedBytes() >= limit) throw IOException("Место для скачанного кончилось: лимит ${app.store.downloadLimitGb} ГБ")
            val playlistUrl = app.api.hlsUrl(t.id)
            val text = http.newCall(Request.Builder().url(playlistUrl).header("User-Agent", USER_AGENT).build()).execute().use { r ->
                if (!r.isSuccessful) throw IOException("плейлист ${r.code}")
                r.body?.string() ?: throw IOException("пустой плейлист")
            }
            val base = playlistUrl.toHttpUrl()
            fun abs(u: String) = base.resolve(u)?.toString() ?: u
            var init: String? = null
            val parts = ArrayList<String>()
            for (line in text.lines().map { it.trim() }) {
                if (line.startsWith("#EXT-X-MAP:")) Regex("URI=\"([^\"]+)\"").find(line)?.let { init = abs(it.groupValues[1]) }
                else if (line.isNotEmpty() && !line.startsWith("#")) parts.add(abs(line))
            }
            if (parts.isEmpty()) throw IOException("в плейлисте нет кусков")
            val f = File(dir(ctx), "${t.id}.${if (init != null) "m4a" else "mp3"}")
            out = f
            f.outputStream().buffered().use { o ->
                val all = (listOfNotNull(init) + parts)
                all.forEachIndexed { i, u ->
                    http.newCall(Request.Builder().url(u).header("User-Agent", USER_AGENT).build()).execute().use { r ->
                        if (!r.isSuccessful) throw IOException("кусок ${r.code}")
                        r.body?.byteStream()?.copyTo(o)
                    }
                    running[t.id] = (i + 1f) / all.size
                    progress((i + 1f) / all.size)
                    if (usedBytes() + f.length() > limit) throw IOException("Место для скачанного кончилось: лимит ${app.store.downloadLimitGb} ГБ")
                }
            }
            saved[t.id] = Saved(t, f.absolutePath, f.length(), System.currentTimeMillis())
            persist(app.store)
        } catch (e: Exception) {
            out?.delete()
            throw e
        } finally {
            running.remove(t.id)
        }
    }

    fun remove(app: App, id: Long) {
        saved.remove(id)?.let { File(it.file).delete() }
        persist(app.store)
    }
}

fun fmtSize(b: Long): String = if (b >= 1_000_000_000) "%.1f ГБ".format(b / 1e9) else "%.0f МБ".format(b / 1e6)
