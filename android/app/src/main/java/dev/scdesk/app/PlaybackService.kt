package dev.scdesk.app

import android.net.Uri
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.okhttp.OkHttpDataSource
import androidx.media3.datasource.ResolvingDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService

/**
 * The player: ExoPlayer inside a media session service, so the music goes on with the screen off, and the
 * notification, the lock screen, headset buttons and the car all work with it. A track is queued as
 * `scdesk://<id>` and only when ExoPlayer opens it the address of its HLS stream is asked from SoundCloud.
 */
@UnstableApi
class PlaybackService : MediaSessionService() {
    private var session: MediaSession? = null

    override fun onCreate() {
        super.onCreate()
        val app = application as App
        val resolver = ResolvingDataSource.Resolver { spec ->
            if (spec.uri.scheme == "scdesk") spec.withUri(Uri.parse(app.api.hlsUrl(spec.uri.host!!.toLong()))) else spec
        }
        // the default source reads files too (the saved tracks), the net goes through the same client
        val factory = ResolvingDataSource.Factory(androidx.media3.datasource.DefaultDataSource.Factory(this, OkHttpDataSource.Factory(http).setUserAgent(USER_AGENT)), resolver)
        val player = ExoPlayer.Builder(this)
            .setMediaSourceFactory(DefaultMediaSourceFactory(factory))
            .setAudioAttributes(AudioAttributes.Builder().setUsage(C.USAGE_MEDIA).setContentType(C.AUDIO_CONTENT_TYPE_MUSIC).build(), true)
            .setHandleAudioBecomingNoisy(true)
            .setWakeMode(C.WAKE_MODE_NETWORK)
            .build()
        // the notification of the player carries the cloud of SC Desk, not the default icon
        setMediaNotificationProvider(androidx.media3.session.DefaultMediaNotificationProvider(this).apply { setSmallIcon(R.drawable.ic_stat_sc) })
        session = MediaSession.Builder(this, player).build()
        // the equalizer goes onto the audio session of the player (it can change when the player is made again)
        app.eq.attach(player.audioSessionId)
        player.addListener(object : androidx.media3.common.Player.Listener {
            override fun onAudioSessionIdChanged(audioSessionId: Int) = app.eq.attach(audioSessionId)

            // the next tracks are looked up while this one plays: a skip then starts at once
            override fun onMediaItemTransition(mediaItem: androidx.media3.common.MediaItem?, reason: Int) = warmNext(player)
            override fun onTimelineChanged(timeline: androidx.media3.common.Timeline, reason: Int) = warmNext(player)
        })
    }

    private var warmed = -1L

    private fun warmNext(player: androidx.media3.common.Player) {
        val app = application as App
        val i = player.currentMediaItemIndex
        val ids = (1..3).mapNotNull { k -> player.takeIf { i + k < it.mediaItemCount }?.getMediaItemAt(i + k)?.mediaId?.toLongOrNull() }
        val first = ids.firstOrNull() ?: return
        if (first == warmed) return
        warmed = first
        Thread { app.api.warm(ids) }.apply { isDaemon = true }.start()
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? = session

    override fun onDestroy() {
        session?.run {
            player.release()
            release()
        }
        session = null
        super.onDestroy()
    }
}
