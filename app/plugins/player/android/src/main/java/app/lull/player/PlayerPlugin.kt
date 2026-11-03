package app.lull.player

import android.app.Activity
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.wifi.WifiManager
import android.os.Handler
import android.os.Looper
import android.webkit.CookieManager
import android.webkit.WebView
import androidx.activity.result.ActivityResult
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import com.google.common.util.concurrent.ListenableFuture
import com.google.zxing.client.android.Intents
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanIntentResult
import com.journeyapps.barcodescanner.ScanOptions

@InvokeArg
class HubArgs {
    lateinit var origin: String
    lateinit var token: String
}

@InvokeArg
class MulticastArgs {
    var on: Boolean = false
}

@InvokeArg
class LoadOptions {
    var at: Double = 0.0
    var autoplay: Boolean = true
}

@InvokeArg
class LoadArgs {
    lateinit var item: Item
    var options: LoadOptions? = null
}

@InvokeArg
class SeekArgs {
    var seconds: Double = 0.0
}

@InvokeArg
class VolumeArgs {
    var volume: Double = 1.0
}

@InvokeArg
class NextArgs {
    var item: Item? = null
}

@InvokeArg
class CrossfadeArgs {
    var seconds: Double = 0.0
}

@InvokeArg
class DownloadAddArgs {
    lateinit var trackId: String
    lateinit var url: String
    var artwork: String? = null
    var meta: JSObject? = null
}

@InvokeArg
class DownloadRemoveArgs {
    var trackIds: List<String> = emptyList()
}

/**
 * The commands behind `window.LullNative` (docs/context/web.md: output interface and downloads).
 * Tauri calls them on its IPC thread; each runs on the main thread, where the players live.
 */
@UnstableApi
@TauriPlugin
class PlayerPlugin(private val activity: Activity) : Plugin(activity) {
    private val main = Handler(Looper.getMainLooper())
    private lateinit var downloads: Downloads
    private var controller: ListenableFuture<MediaController>? = null
    private var multicast: WifiManager.MulticastLock? = null

    override fun load(webView: WebView) {
        Engine.init(activity)
        Engine.emit = { event, payload -> trigger(event, payload) }
        downloads = Downloads(activity) { event, payload -> trigger(event, payload) }
    }

    /** From Rust before the web UI loads: the hub's token for streams, downloads and the WebView. */
    @Command
    fun configure(invoke: Invoke) {
        val args = invoke.parseArgs(HubArgs::class.java)
        onMain(invoke) {
            Engine.configure(args.token)
            downloads.configure(args.token)
            // Tauri can't set cookies on Android: the web UI's requests get the device cookie here.
            val secure = if (args.origin.startsWith("https://")) "; Secure" else ""
            val cookies = CookieManager.getInstance()
            cookies.setCookie(args.origin, "lull_device=${args.token}; Path=/; HttpOnly; SameSite=Lax$secure")
            cookies.flush()
            null
        }
    }

    @Command
    fun multicast(invoke: Invoke) {
        val args = invoke.parseArgs(MulticastArgs::class.java)
        onMain(invoke) {
            if (args.on) {
                val wifi = activity.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
                multicast = multicast ?: wifi.createMulticastLock("lull-discovery").apply { setReferenceCounted(false) }
                multicast?.acquire()
            } else {
                multicast?.release()
            }
            null
        }
    }

    @Command
    fun load(invoke: Invoke) {
        val args = invoke.parseArgs(LoadArgs::class.java)
        onMain(invoke) {
            startService()
            val o = args.options ?: LoadOptions()
            Engine.load(args.item, o.at, o.autoplay)
            null
        }
    }

    @Command
    fun play(invoke: Invoke) = onMain(invoke) { startService(); Engine.play(); null }

    @Command
    fun pause(invoke: Invoke) = onMain(invoke) { Engine.pause(); null }

    @Command
    fun seek(invoke: Invoke) {
        val args = invoke.parseArgs(SeekArgs::class.java)
        onMain(invoke) { Engine.seek(args.seconds); null }
    }

    @Command
    fun setVolume(invoke: Invoke) {
        val args = invoke.parseArgs(VolumeArgs::class.java)
        onMain(invoke) { Engine.setVolume(args.volume); null }
    }

    @Command
    fun setNext(invoke: Invoke) {
        val args = invoke.parseArgs(NextArgs::class.java)
        onMain(invoke) { Engine.setNext(args.item); null }
    }

    @Command
    fun crossfade(invoke: Invoke) {
        val args = invoke.parseArgs(CrossfadeArgs::class.java)
        onMain(invoke) { Engine.crossfade(args.seconds); null }
    }

    @Command
    fun downloadAdd(invoke: Invoke) {
        val args = invoke.parseArgs(DownloadAddArgs::class.java)
        onMain(invoke) { downloads.add(args.trackId, args.url, args.artwork, args.meta); null }
    }

    @Command
    fun downloadRemove(invoke: Invoke) {
        val args = invoke.parseArgs(DownloadRemoveArgs::class.java)
        onMain(invoke) { downloads.remove(args.trackIds); null }
    }

    @Command
    fun downloadList(invoke: Invoke) = onMain(invoke) { JSObject().put("items", downloads.list()) }

    @Command
    fun downloadUsage(invoke: Invoke) = onMain(invoke) { downloads.usage() }

    /**
     * The connect page's QR scanner: ZXing's camera screen, which asks for the camera itself. Resolves
     * `{ content }`, null when the listener backs out.
     */
    @Command
    fun scan(invoke: Invoke) {
        val options = ScanOptions()
            .setDesiredBarcodeFormats(ScanOptions.QR_CODE)
            .setPrompt("Point the camera at the hub's pairing QR code")
            .setBeepEnabled(false)
            .setOrientationLocked(false)
        startActivityForResult(invoke, ScanContract().createIntent(activity, options), "scanned")
    }

    @ActivityCallback
    private fun scanned(invoke: Invoke, result: ActivityResult) {
        val scan = ScanIntentResult.parseActivityResult(result.resultCode, result.data)
        if (scan.originalIntent?.getBooleanExtra(Intents.Scan.MISSING_CAMERA_PERMISSION, false) == true) {
            invoke.reject("lull can't use the camera. Allow it in Android's settings, or type the pair code.")
            return
        }
        invoke.resolve(JSObject().put("content", scan.contents))
    }

    /** Binding a controller starts the MediaSession service, which then keeps playback alive. */
    private fun startService() {
        if (controller != null) return
        val token = SessionToken(activity, ComponentName(activity, PlaybackService::class.java))
        controller = MediaController.Builder(activity, token).buildAsync()
    }

    /** Runs `work` on the main thread; resolves with what it returns, or rejects with its message. */
    private fun onMain(invoke: Invoke, work: () -> JSObject?) {
        main.post {
            try {
                val result = work()
                if (result == null) invoke.resolve() else invoke.resolve(result)
            } catch (e: Exception) {
                invoke.reject(e.message ?: e.toString())
            }
        }
    }
}
