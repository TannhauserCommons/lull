package app.lull.player

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.database.ContentObserver
import android.media.AudioManager
import android.os.Handler
import android.os.Looper
import android.provider.Settings

/**
 * The phone asks for quiet (ADR-0081): its ringer is on silent or vibrate, or touch sounds are off.
 * The web UI's sounds rest then. Followed for the life of the process from the application context,
 * like the engine; every change goes to `emit`, on the main thread.
 */
object SoundQuiet {
    var emit: (Boolean) -> Unit = {}
    private var context: Context? = null
    private var last = false

    /** Starts following, once; a later activity only takes the events (`emit`). */
    fun init(activity: Context) {
        if (context != null) return
        val app = activity.applicationContext
        context = app
        last = now()
        // A system broadcast: no exported flag is needed on Android 14 and later.
        app.registerReceiver(
            object : BroadcastReceiver() {
                override fun onReceive(context: Context, intent: Intent) = recheck()
            },
            IntentFilter(AudioManager.RINGER_MODE_CHANGED_ACTION),
        )
        app.contentResolver.registerContentObserver(
            Settings.System.getUriFor(Settings.System.SOUND_EFFECTS_ENABLED),
            false,
            object : ContentObserver(Handler(Looper.getMainLooper())) {
                override fun onChange(selfChange: Boolean) = recheck()
            },
        )
    }

    /** Whether the phone asks for quiet now. */
    fun now(): Boolean {
        val app = context ?: return false
        val audio = app.getSystemService(Context.AUDIO_SERVICE) as AudioManager
        val touchSounds = Settings.System.getInt(app.contentResolver, Settings.System.SOUND_EFFECTS_ENABLED, 1) != 0
        return audio.ringerMode != AudioManager.RINGER_MODE_NORMAL || !touchSounds
    }

    private fun recheck() {
        val quiet = now()
        if (quiet == last) return
        last = quiet
        emit(quiet)
    }
}
