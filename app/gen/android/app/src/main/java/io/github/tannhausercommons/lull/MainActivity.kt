package io.github.tannhausercommons.lull

import android.content.Intent
import android.os.Bundle
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  private var webView: WebView? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    SharedLinks.receive(intent)
  }

  // singleTask: a link shared while lull runs comes here; the page takes it at once.
  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    SharedLinks.receive(intent)
    webView?.let { SharedLinks.nudge(it) }
  }

  override fun onWebViewCreate(webView: WebView) {
    this.webView = webView
    webView.addJavascriptInterface(SharedLinks, "lullLinks")
  }
}

/**
 * A song, album, artist or playlist shared to lull: "Share" from another app (its text holds the
 * link) or a lull://open?link=… link. Held until the hub's web UI takes it (App.takeSharedLink,
 * at boot and when nudged), which opens it as #/open; a bundled page never asks.
 */
object SharedLinks {
  private val WEB_LINK = Regex("https?://\\S+")

  @Volatile private var pending: String? = null

  fun receive(intent: Intent?) {
    val link = when (intent?.action) {
      Intent.ACTION_VIEW -> intent.data?.takeIf { it.scheme == "lull" && it.host == "open" }?.getQueryParameter("link")
      Intent.ACTION_SEND -> intent.getStringExtra(Intent.EXTRA_TEXT)?.let { WEB_LINK.find(it)?.value }
      else -> null
    }
    if (link != null && WEB_LINK.matches(link)) pending = link
  }

  @JavascriptInterface
  fun take(): String {
    val link = pending ?: ""
    pending = null
    return link
  }

  fun nudge(webView: WebView) {
    webView.post { webView.evaluateJavascript("window.App && App.takeSharedLink && App.takeSharedLink()", null) }
  }
}
