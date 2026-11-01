package app.lull.player

import android.content.Context
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.DefaultDataSource
import androidx.media3.datasource.DefaultHttpDataSource
import androidx.media3.datasource.HttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.session.MediaSession
import app.tauri.annotation.InvokeArg
import app.tauri.plugin.JSObject
import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.min
import kotlin.math.pow
import kotlin.math.sin

/** One queue entry from the web UI (docs/context/web.md, output interface). */
@InvokeArg
class Item {
    lateinit var key: String
    lateinit var url: String
    var trackId: String? = null
    var title: String? = null
    var artist: String? = null
    var album: String? = null
    var duration: Double? = null
    var artwork: String? = null
    var gainDb: Double? = null
}

/**
 * The web UI's output contract on two ExoPlayer decks. One deck plays; with crossfade off the next
 * item waits in the same deck's playlist (ExoPlayer plays that gaplessly), with crossfade on it
 * waits paused in the other deck, which fades in over the current one's last seconds. Main thread
 * only: the plugin posts every call here.
 */
@UnstableApi
object Engine {
    /** How often `time` is sent while playing (the contract asks for at least 4 a second). */
    private const val TICK_MS = 250L

    /** An end earlier than this before the duration is a cut response, not the song's end. */
    private const val CUT_SLACK_MS = 2_000L

    private val main = Handler(Looper.getMainLooper())
    private lateinit var context: Context
    private val http = DefaultHttpDataSource.Factory()
        .setAllowCrossProtocolRedirects(true)
        .setUserAgent("lull-app")

    /** Where events go: the plugin's `trigger`. */
    var emit: (String, JSObject) -> Unit = { _, _ -> }

    private val decks = arrayOfNulls<ExoPlayer>(2)
    private var active = 0
    private var current: Item? = null
    private var next: Item? = null
    private var fadeSeconds = 0.0
    private var volume = 1f
    private var fade: Fade? = null
    private var ticking = false

    /** What the active deck last reported, to tell a song's end from a cut response. */
    private var lastPositionMs = 0L
    private var lastDurationMs = C.TIME_UNSET
    private var lastBufferedMs = -1L

    private var session: MediaSession? = null

    private class Fade(val from: Item, val to: Item, val startMs: Long, val lengthMs: Long)

    private val music: AudioAttributes = AudioAttributes.Builder()
        .setUsage(C.USAGE_MEDIA)
        .setContentType(C.AUDIO_CONTENT_TYPE_MUSIC)
        .build()

    val sources get() = DefaultDataSource.Factory(context, http)

    fun init(context: Context) {
        if (this::context.isInitialized) return
        this.context = context.applicationContext
        for (i in decks.indices) decks[i] = deck()
        updateFocus()
    }

    /** Streams and covers come from the hub with this device's token. */
    fun configure(token: String) {
        http.setDefaultRequestProperties(mapOf("Authorization" to "Bearer $token"))
    }

    /** The MediaSession service's player: whichever deck plays (PlaybackService). */
    fun attach(session: MediaSession) {
        this.session = session
    }

    fun detach() {
        session = null
    }

    fun sessionPlayer(): Player = SessionPlayer(activeDeck())

    val isPlaying get() = activeDeck().isPlaying || fade != null

    // ----- the contract's methods -----

    fun load(item: Item, atSeconds: Double, autoplay: Boolean) {
        fade = null
        clear(idleDeck())
        next = null
        current = item
        val deck = activeDeck()
        lastPositionMs = (atSeconds * 1000).toLong()
        lastDurationMs = C.TIME_UNSET
        lastBufferedMs = -1L
        deck.setMediaItem(mediaItem(item), lastPositionMs)
        deck.prepare()
        deck.playWhenReady = autoplay
        applyVolume()
        sendTime()
    }

    fun play() {
        activeDeck().play()
    }

    fun pause() {
        abandonFade()
        activeDeck().pause()
    }

    fun seek(seconds: Double) {
        abandonFade()
        activeDeck().seekTo((seconds * 1000).toLong())
    }

    fun setVolume(v: Double) {
        volume = v.toFloat().coerceIn(0f, 1f)
        applyVolume()
    }

    fun setNext(item: Item?) {
        next = item
        // During a fade the target is already playing; the web UI sets the next one after `ended`.
        if (fade == null) arrangeNext()
    }

    fun crossfade(seconds: Double) {
        fadeSeconds = seconds.coerceAtLeast(0.0)
        if (fade == null) arrangeNext()
    }

    /** Lock screen, notification, Bluetooth or headset next/prev: the web UI owns the queue. */
    fun remote(action: String) {
        emit("remote", JSObject().put("action", action))
    }

    // ----- decks -----

    private fun deck(): ExoPlayer {
        val player = ExoPlayer.Builder(context)
            .setMediaSourceFactory(DefaultMediaSourceFactory(sources))
            .setHandleAudioBecomingNoisy(true)
            .setWakeMode(C.WAKE_MODE_NETWORK)
            .build()
        player.addListener(DeckListener(player))
        return player
    }

    private fun activeDeck(): ExoPlayer = decks[active]!!
    private fun idleDeck(): ExoPlayer = decks[1 - active]!!

    private fun clear(deck: ExoPlayer) {
        deck.stop()
        deck.clearMediaItems()
    }

    /** Only the playing deck holds audio focus, so the other one never ducks or pauses it. */
    private fun updateFocus() {
        decks.forEachIndexed { i, deck -> deck?.setAudioAttributes(music, i == active) }
    }

    /** Puts the next item where the current mode wants it: the playlist (gapless) or the idle deck (crossfade). */
    private fun arrangeNext() {
        val deck = activeDeck()
        while (deck.mediaItemCount > deck.currentMediaItemIndex + 1) deck.removeMediaItem(deck.mediaItemCount - 1)
        clear(idleDeck())
        val item = next ?: return
        if (fadeSeconds > 0) {
            val idle = idleDeck()
            idle.volume = 0f
            idle.setMediaItem(mediaItem(item))
            idle.prepare()
            idle.playWhenReady = false
        } else {
            deck.addMediaItem(mediaItem(item))
        }
    }

    private fun mediaItem(item: Item): MediaItem {
        val meta = MediaMetadata.Builder()
            .setTitle(item.title)
            .setArtist(item.artist)
            .setAlbumTitle(item.album)
            .setArtworkUri(item.artwork?.let(Uri::parse))
            .build()
        return MediaItem.Builder().setMediaId(item.key).setUri(item.url).setMediaMetadata(meta).build()
    }

    /** ReplayGain as a level: `volume × 10^(gain/20)`, at most full scale (players can't amplify). */
    private fun level(item: Item?): Float {
        val gain = item?.gainDb?.let { 10.0.pow(it / 20.0) } ?: 1.0
        return (volume * gain).toFloat().coerceIn(0f, 1f)
    }

    private fun applyVolume() {
        val f = fade
        if (f == null) {
            activeDeck().volume = level(current)
            return
        }
        val t = progress(f)
        activeDeck().volume = level(f.from) * cos(t * PI / 2).toFloat()
        idleDeck().volume = level(f.to) * sin(t * PI / 2).toFloat()
    }

    // ----- crossfade -----

    private fun progress(f: Fade): Double =
        ((SystemClock.elapsedRealtime() - f.startMs).toDouble() / f.lengthMs).coerceIn(0.0, 1.0)

    private fun startFade(remainingMs: Long) {
        val from = current ?: return
        val to = next ?: return
        fade = Fade(from, to, SystemClock.elapsedRealtime(), min((fadeSeconds * 1000).toLong(), remainingMs).coerceAtLeast(1))
        applyVolume()
        idleDeck().play()
        emit("crossfade", JSObject().put("key", from.key).put("next", to.key))
    }

    /** The fade is done (or the old song ended first): the other deck is the active one now. */
    private fun finishFade() {
        val f = fade ?: return
        fade = null
        clear(activeDeck())
        active = 1 - active
        current = f.to
        next = null
        lastDurationMs = C.TIME_UNSET
        lastBufferedMs = -1L
        updateFocus()
        applyVolume()
        session?.player = sessionPlayer()
        emit("ended", JSObject().put("key", f.from.key).put("next", f.to.key))
        sendTime()
    }

    /** Pause or seek during a fade: back to the current song alone, the next one waiting at its start. */
    private fun abandonFade() {
        val f = fade ?: return
        fade = null
        idleDeck().pause()
        idleDeck().seekTo(0)
        idleDeck().volume = 0f
        applyVolume()
        emit("crossfade", JSObject().put("key", f.from.key).put("next", JSONObject.NULL))
    }

    // ----- events -----

    private val tick = object : Runnable {
        override fun run() {
            val deck = activeDeck()
            if (current != null) {
                lastPositionMs = deck.currentPosition
                durationMs(deck)?.let { lastDurationMs = it }
                sendTime()
                sendBuffered(deck)
                val f = fade
                if (f != null) {
                    if (progress(f) >= 1.0) finishFade() else applyVolume()
                } else if (fadeSeconds > 0 && next != null && deck.isPlaying) {
                    val left = (lastDurationMs.takeIf { it != C.TIME_UNSET } ?: Long.MAX_VALUE) - lastPositionMs
                    if (left <= fadeSeconds * 1000) startFade(left)
                }
            }
            if (isPlaying) main.postDelayed(this, TICK_MS) else ticking = false
        }
    }

    private fun startTicking() {
        if (ticking) return
        ticking = true
        main.post(tick)
    }

    private fun durationMs(deck: ExoPlayer): Long? {
        val d = deck.duration
        if (d != C.TIME_UNSET) return d
        return current?.duration?.let { (it * 1000).toLong() }
    }

    private fun sendTime() {
        val item = current ?: return
        val deck = activeDeck()
        val ended = deck.playbackState == Player.STATE_ENDED || deck.playbackState == Player.STATE_IDLE
        val duration = durationMs(deck)
        emit(
            "time",
            JSObject()
                .put("key", item.key)
                .put("position", deck.currentPosition / 1000.0)
                .put("duration", duration?.let { it / 1000.0 } ?: JSONObject.NULL)
                .put("playing", deck.playWhenReady && !ended)
                .put("waiting", deck.playWhenReady && deck.playbackState == Player.STATE_BUFFERING),
        )
    }

    private fun sendBuffered(deck: ExoPlayer) {
        val item = current ?: return
        val buffered = deck.bufferedPosition
        if (kotlin.math.abs(buffered - lastBufferedMs) < 500) return
        lastBufferedMs = buffered
        val range = JSONArray().put(0).put(buffered / 1000.0)
        emit("buffered", JSObject().put("key", item.key).put("ranges", JSONArray().put(range)))
    }

    private class DeckListener(private val deck: ExoPlayer) : Player.Listener {
        private val isActive get() = deck === activeDeck()

        override fun onIsPlayingChanged(isPlaying: Boolean) {
            if (!isActive) return
            sendTime()
            if (isPlaying) startTicking()
        }

        override fun onPlayWhenReadyChanged(playWhenReady: Boolean, reason: Int) {
            if (isActive) sendTime()
        }

        override fun onPlaybackStateChanged(state: Int) {
            if (!isActive) return
            sendTime()
            if (state != Player.STATE_ENDED) return
            val item = current ?: return
            if (fade != null) return finishFade()
            emit("ended", JSObject().put("key", item.key).put("next", JSONObject.NULL))
        }

        /** Gapless: the playlist moved into the next item, unless the current response was cut short. */
        override fun onMediaItemTransition(mediaItem: MediaItem?, reason: Int) {
            if (!isActive || reason != Player.MEDIA_ITEM_TRANSITION_REASON_AUTO) return
            val old = current ?: return
            val cut = lastDurationMs != C.TIME_UNSET && lastPositionMs < lastDurationMs - CUT_SLACK_MS
            if (cut) {
                // The web UI reopens the cut song with `load` and sets the next one again.
                clear(deck)
                emit("ended", JSObject().put("key", old.key).put("next", JSONObject.NULL))
                return
            }
            val now = next ?: return
            deck.removeMediaItem(0)
            current = now
            next = null
            lastDurationMs = C.TIME_UNSET
            lastBufferedMs = -1L
            applyVolume()
            emit("ended", JSObject().put("key", old.key).put("next", now.key))
            sendTime()
        }

        override fun onPlayerError(error: PlaybackException) {
            val key = (if (isActive) current else next)?.key ?: return
            val status = (error.cause as? HttpDataSource.InvalidResponseCodeException)?.responseCode
            val midSong = isActive && status == null && lastPositionMs > 0 &&
                error.errorCode in PlaybackException.ERROR_CODE_IO_UNSPECIFIED until PlaybackException.ERROR_CODE_PARSING_CONTAINER_MALFORMED
            if (midSong) {
                // A response that broke off mid-song is a cut, which the web UI reopens (contract: `ended`).
                emit("ended", JSObject().put("key", key).put("next", JSONObject.NULL))
                return
            }
            emit(
                "error",
                JSObject()
                    .put("key", key)
                    .put("message", error.message ?: "This song can't play")
                    .put("status", status ?: JSONObject.NULL),
            )
        }
    }

    /**
     * What the MediaSession sees: the active deck, except that next/prev go to the web UI (it owns
     * the queue) and play/pause go through the engine (so a fade stops with them).
     */
    private class SessionPlayer(player: Player) : androidx.media3.common.ForwardingPlayer(player) {
        private val extra = intArrayOf(
            Player.COMMAND_SEEK_TO_NEXT,
            Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM,
            Player.COMMAND_SEEK_TO_PREVIOUS,
            Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM,
        )

        override fun getAvailableCommands(): Player.Commands =
            super.getAvailableCommands().buildUpon().addAll(*extra).build()

        override fun isCommandAvailable(command: Int): Boolean =
            command in extra || super.isCommandAvailable(command)

        override fun hasNextMediaItem() = true
        override fun hasPreviousMediaItem() = true
        override fun seekToNext() = remote("next")
        override fun seekToNextMediaItem() = remote("next")
        override fun seekToPrevious() = remote("prev")
        override fun seekToPreviousMediaItem() = remote("prev")
        override fun play() = Engine.play()
        override fun pause() = Engine.pause()
        override fun setPlayWhenReady(playWhenReady: Boolean) = if (playWhenReady) Engine.play() else Engine.pause()
    }
}
