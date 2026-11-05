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
 * A song, album, artist or playlist shared to lull: an App Link (a share host's page, whose address
 * names the item, ADR-0034, or lull's older open page, whose `#lull?` part does, ADR-0032; the
 * whole address either way), "Share" from another app (its text holds the
 * link) or a lull://open?link=… link, which may also ask lull to add it to the queue (`do=queue`,
 * from a share page's Add to queue). Held until the hub's web UI takes it (App.takeSharedLink, at
 * boot and when nudged), which opens it as #/open; a bundled page never asks. `take` answers the
 * link alone, as before, so a hub's older web UI still opens it; `takeAction` answers what was asked.
 */
object SharedLinks {
  private val WEB_LINK = Regex("https?://\\S+")

  @Volatile private var pending: String? = null
  @Volatile private var pendingAction: String = ""

  fun receive(intent: Intent?) {
    var action = ""
    val link = when (intent?.action) {
      Intent.ACTION_VIEW -> intent.data?.let {
        when {
          // Only the manifest's App Links filters (the open page, the share hosts) send https links
          // here, so the host needs no second list; the hub reads the link and refuses anything else.
          it.scheme == "https" -> it.toString()
          it.scheme == "lull" && it.host == "open" -> {
            if (it.getQueryParameter("do") == "queue") action = "queue"
            it.getQueryParameter("link")
          }
          else -> null
        }
      }
      Intent.ACTION_SEND -> intent.getStringExtra(Intent.EXTRA_TEXT)?.let { WEB_LINK.find(it)?.value }
      else -> null
    }
    if (link != null && WEB_LINK.matches(link)) {
      pending = link
      pendingAction = action
    }
  }

  @JavascriptInterface
  fun take(): String {
    val link = pending ?: ""
    pending = null
    return link
  }

  /** What the link that `take` answered asks for: "queue", or "" to open it. */
  @JavascriptInterface
  fun takeAction(): String {
    val action = pendingAction
    pendingAction = ""
    return action
  }

  fun nudge(webView: WebView) {
    webView.post { webView.evaluateJavascript("window.App && App.takeSharedLink && App.takeSharedLink()", null) }
  }
}
