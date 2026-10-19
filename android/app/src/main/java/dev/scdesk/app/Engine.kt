package dev.scdesk.app

import android.content.ComponentName
import android.content.Context
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import androidx.core.content.ContextCompat
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.MimeTypes
import androidx.media3.common.Player
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import kotlinx.coroutines.flow.MutableStateFlow

data class PlayState(
    val track: Track? = null,
    val playing: Boolean = false,
    val loading: Boolean = false,
    val durationMs: Long = 0,
    val volume: Float = 1f,
    val hasNext: Boolean = false,
    val shuffle: Boolean = false,
    /** 0 off, 1 the whole queue, 2 one track */
    val repeat: Int = 0,
)

/** What the screens and the link talk to: the player of the service (PlaybackService), seen through a controller. */
class Engine(private val ctx: Context) {
    val state = MutableStateFlow(PlayState())

    /** the queue was replaced: true when by the wave (so the wave knows when it is not the one that plays) */
    var onQueueReplaced: ((Boolean) -> Unit)? = null

    /** something changed in the player: the link tells the other devices */
    var onChanged: (() -> Unit)? = null
    /** a track could not be played: what to tell the user */
    var onError: ((String) -> Unit)? = null
    /** a track started to play here (once per start) */
    var onStarted: ((Track) -> Unit)? = null
    private var lastStarted = -1L

    private val main = Handler(Looper.getMainLooper())
    private var controller: MediaController? = null
    private var connecting = false
    private val waiting = ArrayList<(MediaController) -> Unit>()

    private val listener = object : Player.Listener {
        override fun onEvents(player: Player, events: Player.Events) = refresh()

        override fun onPlayerError(error: androidx.media3.common.PlaybackException) {
            val c = controller ?: return
            val t = trackOf(c.currentMediaItem)
            var why = error.message ?: error.errorCodeName
            var cause: Throwable? = error
            while (cause != null) { cause.message?.let { why = it }; cause = cause.cause }
            val text = when {
                why.contains("no stream", true) || why.contains("not resolved", true) || why.contains("no media", true) -> "для этого трека нет потока: SoundCloud его не отдаёт (закрыт или недоступен в вашей стране)"
                why.contains("403") || why.contains("401") -> "SoundCloud отказал в доступе к потоку"
                why.contains("404") -> "трек удалён или недоступен"
                else -> why.take(90)
            }
            android.util.Log.w("SCDesk", "play error: " + error.errorCodeName + " " + why)
            onError?.invoke("Не удалось включить «" + (t?.title ?: "трек") + "»: " + text)
            // the next track plays instead of silence
            if (c.hasNextMediaItem()) { c.seekToNextMediaItem(); c.prepare(); c.play() }
        }
    }

    fun connect() {
        if (controller != null || connecting) return
        connecting = true
        val token = SessionToken(ctx, ComponentName(ctx, PlaybackService::class.java))
        val future = MediaController.Builder(ctx, token).buildAsync()
        future.addListener({
            try {
                val c = future.get()
                controller = c
                c.addListener(listener)
                waiting.forEach { it(c) }
                waiting.clear()
                refresh()
            } catch (e: Exception) {
                // the service did not start: the next command tries again
            } finally {
                connecting = false
            }
        }, ContextCompat.getMainExecutor(ctx))
    }

    /** Runs [block] on the main thread with the controller (as soon as it is there). */
    fun run(block: (MediaController) -> Unit) {
        main.post {
            val c = controller
            if (c != null) block(c) else {
                waiting.add(block)
                connect()
            }
        }
    }

    private fun trackOf(item: MediaItem?): Track? = item?.let {
        val m = it.mediaMetadata
        Track(it.mediaId.toLongOrNull() ?: return null, m.title?.toString() ?: "", m.artist?.toString() ?: "", m.extras?.getLong("dur") ?: 0, m.artworkUri?.toString())
    }

    private fun refresh() {
        val c = controller ?: return
        val buffering = c.playbackState == Player.STATE_BUFFERING
        val t = trackOf(c.currentMediaItem)
        if (t != null && c.isPlaying && t.id != lastStarted) {
            lastStarted = t.id
            onStarted?.invoke(t)
        } else if (t == null) lastStarted = -1L
        state.value = PlayState(
            track = t,
            playing = c.isPlaying || (c.playWhenReady && buffering),
            loading = buffering,
            durationMs = if (c.duration > 0) c.duration else t?.durationMs ?: 0,
            volume = c.volume,
            hasNext = c.hasNextMediaItem(),
            shuffle = c.shuffleModeEnabled,
            repeat = when (c.repeatMode) { Player.REPEAT_MODE_ALL -> 1; Player.REPEAT_MODE_ONE -> 2; else -> 0 },
        )
        onChanged?.invoke()
    }

    private fun item(t: Track): MediaItem {
        // a saved track plays from its file, the others from the stream
        val local = Downloads.localFile(t.id)
        return MediaItem.Builder()
            .setMediaId(t.id.toString())
            .setUri(if (local != null) Uri.fromFile(local) else Uri.parse("scdesk://${t.id}"))
            .setMimeType(if (local != null) Downloads.mimeOf(local) else MimeTypes.APPLICATION_M3U8)
        .setMediaMetadata(
            MediaMetadata.Builder().setTitle(t.title).setArtist(t.artist)
                .setArtworkUri(t.artwork?.let(Uri::parse))
                .setExtras(Bundle().apply { putLong("dur", t.durationMs) })
                .build(),
        )
        .build()
    }

    fun playQueue(tracks: List<Track>, index: Int = 0, positionMs: Long = 0, play: Boolean = true, wave: Boolean = false) {
        if (tracks.isEmpty()) return
        onQueueReplaced?.invoke(wave)
        run { c ->
            c.setMediaItems(tracks.map(::item), index.coerceIn(0, tracks.size - 1), positionMs)
            c.prepare()
            c.playWhenReady = play
        }
    }

    /** How fast the music plays (the pitch stays), 1 is usual. */
    fun applySpeed(value: Float) = run { c -> c.setPlaybackSpeed(value) }
    @Volatile var speed = 1f

    /** This track right after the playing one. */
    fun insertNext(t: Track) = run { c -> c.addMediaItem((c.currentMediaItemIndex + 1).coerceAtMost(c.mediaItemCount), item(t)) }

    /** More tracks at the end of the queue (the wave goes on). */
    fun append(tracks: List<Track>) = run { c -> c.addMediaItems(tracks.map(::item)) }

    /** How many tracks follow the playing one. */
    fun remaining(cb: (Int) -> Unit) = run { c -> cb(c.mediaItemCount - c.currentMediaItemIndex - 1) }

    fun toggleShuffle() = run { it.shuffleModeEnabled = !it.shuffleModeEnabled }
    fun cycleRepeat() = run { it.repeatMode = when (it.repeatMode) { Player.REPEAT_MODE_OFF -> Player.REPEAT_MODE_ALL; Player.REPEAT_MODE_ALL -> Player.REPEAT_MODE_ONE; else -> Player.REPEAT_MODE_OFF } }

    /** The queue (what is played, and from where), given on the main thread. */
    fun queue(cb: (List<Track>, Int) -> Unit) = run { c -> cb((0 until c.mediaItemCount).mapNotNull { trackOf(c.getMediaItemAt(it)) }, c.currentMediaItemIndex) }
    fun jump(index: Int) = run { it.seekToDefaultPosition(index); it.play() }

    /** Takes a track out of the queue (not the one that plays). */
    fun removeId(id: Long) = run { c -> for (i in c.mediaItemCount - 1 downTo 0) if (c.getMediaItemAt(i).mediaId == id.toString() && i != c.currentMediaItemIndex) c.removeMediaItem(i) }

    fun toggle() = run { if (it.isPlaying || it.playWhenReady) it.pause() else it.play() }
    fun play() = run { it.play() }
    fun pause() = run { it.pause() }
    fun next() = run { it.seekToNextMediaItem() }
    fun prev() = run { it.seekToPreviousMediaItem() }
    fun seek(ms: Long) = run { it.seekTo(ms) }
    fun volume(v: Float) = run { it.volume = v.coerceIn(0f, 1f) }

    /** Main thread only: where the playing track is. */
    fun position(): Long = controller?.currentPosition ?: 0

    /** Main thread only: the id of the playing track and of those that follow (to hand them to another device). */
    fun queueFromHere(max: Int): Pair<List<Long>, Long> {
        val c = controller ?: return emptyList<Long>() to 0
        val ids = ArrayList<Long>()
        var i = c.currentMediaItemIndex
        while (i < c.mediaItemCount && ids.size < max) {
            c.getMediaItemAt(i).mediaId.toLongOrNull()?.let(ids::add)
            i++
        }
        return ids to c.currentPosition
    }
}
