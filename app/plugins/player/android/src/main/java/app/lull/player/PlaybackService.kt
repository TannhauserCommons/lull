package app.lull.player

import android.app.PendingIntent
import android.content.Intent
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.DataSourceBitmapLoader
import androidx.media3.session.CacheBitmapLoader
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService

/**
 * Keeps lull playing with the screen off, and gives the lock screen, notification, Bluetooth and
 * headset their controls. The engine's active deck is the session's player; Media3 runs the
 * foreground notification while it plays.
 */
@UnstableApi
class PlaybackService : MediaSessionService() {
    private var session: MediaSession? = null

    override fun onCreate() {
        super.onCreate()
        Engine.init(this)
        // Covers come from the hub with the device's token, like the streams.
        val covers = CacheBitmapLoader(
            DataSourceBitmapLoader(DataSourceBitmapLoader.DEFAULT_EXECUTOR_SERVICE.get(), Engine.sources),
        )
        val builder = MediaSession.Builder(this, Engine.sessionPlayer()).setBitmapLoader(covers)
        packageManager.getLaunchIntentForPackage(packageName)?.let { open ->
            builder.setSessionActivity(PendingIntent.getActivity(this, 0, open, PendingIntent.FLAG_IMMUTABLE))
        }
        session = builder.build().also(Engine::attach)
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? = session

    /** Swiping lull away while paused ends it; while playing, the music goes on. */
    override fun onTaskRemoved(rootIntent: Intent?) {
        if (!Engine.isPlaying) stopSelf()
    }

    override fun onDestroy() {
        Engine.detach()
        session?.release()
        session = null
        super.onDestroy()
    }
}
