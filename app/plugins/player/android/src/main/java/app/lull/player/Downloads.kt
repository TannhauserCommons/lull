package app.lull.player

import android.app.DownloadManager
import android.content.Context
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.os.StatFs
import app.tauri.plugin.JSObject
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import kotlin.concurrent.thread

/**
 * Offline downloads in app storage (ADR-0005, docs/context/web.md). Android's DownloadManager
 * fetches the audio, so it goes on in the background and after lull is closed; the cover and the
 * track's JSON sit next to it. `index.json` remembers each track's download. Main thread only.
 */
class Downloads(context: Context, private val emit: (String, JSObject) -> Unit) {
    private val manager = context.getSystemService(Context.DOWNLOAD_SERVICE) as DownloadManager
    // DownloadManager can only write to external app storage; covers and the index stay beside it.
    private val dir = File(context.getExternalFilesDir(null) ?: context.filesDir, "downloads").apply { mkdirs() }
    private val art = File(context.filesDir, "lull-art").apply { mkdirs() }
    private val indexFile = File(dir, "index.json")
    private val index: JSONObject = runCatching { JSONObject(indexFile.readText()) }.getOrDefault(JSONObject())
    private val main = Handler(Looper.getMainLooper())
    private var polling = false
    private var token: String? = null

    fun configure(token: String) {
        this.token = token
    }

    fun add(trackId: String, url: String, artwork: String?, meta: JSONObject?) {
        val known = index.optJSONObject(trackId)
        if (known != null && known.optString("state") != "failed") return
        val file = File(dir, safe(trackId))
        file.delete()
        val request = DownloadManager.Request(Uri.parse(url))
            .setDestinationUri(Uri.fromFile(file))
            .setTitle(meta?.optString("title")?.takeIf { it.isNotEmpty() } ?: "lull")
            .setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE)
        token?.let { request.addRequestHeader("Authorization", "Bearer $it") }
        val entry = JSONObject()
            .put("state", "downloading")
            .put("id", manager.enqueue(request))
            .put("file", file.absolutePath)
            .put("meta", meta ?: JSONObject.NULL)
        artwork?.let { fetchCover(trackId, it) }
        index.put(trackId, entry)
        save()
        poll()
    }

    fun remove(trackIds: List<String>) {
        for (id in trackIds) {
            val entry = index.optJSONObject(id) ?: continue
            manager.remove(entry.optLong("id"))
            File(entry.optString("file")).delete()
            File(art, safe(id)).delete()
            index.remove(id)
        }
        save()
    }

    fun list(): JSONArray {
        refresh()
        val items = JSONArray()
        for (id in index.keys()) items.put(describe(id, index.getJSONObject(id)))
        return items
    }

    fun usage(): JSObject {
        val files = dir.listFiles().orEmpty().toList() + art.listFiles().orEmpty().toList()
        val used = files.fold(0L) { total, file -> total + file.length() }
        return JSObject().put("usedBytes", used).put("freeBytes", StatFs(dir.path).availableBytes)
    }

    /** `artPath` becomes `artUrl` in the app's bridge (Tauri's asset protocol). */
    private fun describe(id: String, entry: JSONObject): JSObject {
        val file = File(entry.optString("file"))
        val cover = File(art, safe(id))
        return JSObject()
            .put("trackId", id)
            .put("state", entry.optString("state"))
            .put("fileUrl", Uri.fromFile(file).toString())
            .put("artPath", if (cover.isFile) cover.absolutePath else JSONObject.NULL)
            .put("sizeBytes", if (file.isFile) file.length() else 0L)
            .put("meta", entry.opt("meta") ?: JSONObject.NULL)
            .put("error", entry.opt("error") ?: JSONObject.NULL)
    }

    /** Progress while anything downloads: once a second, from DownloadManager's own records. */
    private fun poll() {
        if (polling) return
        polling = true
        main.post(object : Runnable {
            override fun run() {
                if (refresh()) main.postDelayed(this, 1_000) else polling = false
            }
        })
    }

    /** Brings `index` up to date with DownloadManager; true while something still downloads. */
    private fun refresh(): Boolean {
        var busy = false
        var changed = false
        for (id in index.keys().asSequence().toList()) {
            val entry = index.getJSONObject(id)
            if (entry.optString("state") != "downloading") continue
            manager.query(DownloadManager.Query().setFilterById(entry.optLong("id"))).use { c ->
                if (!c.moveToFirst()) {
                    fail(id, entry, "The download disappeared")
                    changed = true
                    return@use
                }
                val status = c.getInt(c.getColumnIndexOrThrow(DownloadManager.COLUMN_STATUS))
                val received = c.getLong(c.getColumnIndexOrThrow(DownloadManager.COLUMN_BYTES_DOWNLOADED_SO_FAR))
                val size = c.getLong(c.getColumnIndexOrThrow(DownloadManager.COLUMN_TOTAL_SIZE_BYTES))
                when (status) {
                    DownloadManager.STATUS_SUCCESSFUL -> {
                        entry.put("state", "done")
                        changed = true
                        val d = describe(id, entry)
                        emit("download-done", JSObject().put("trackId", id).put("fileUrl", d.getString("fileUrl"))
                            .put("artPath", d.opt("artPath")).put("sizeBytes", d.getLong("sizeBytes")))
                    }
                    DownloadManager.STATUS_FAILED -> {
                        val reason = c.getInt(c.getColumnIndexOrThrow(DownloadManager.COLUMN_REASON))
                        fail(id, entry, failure(reason))
                        changed = true
                    }
                    else -> {
                        busy = true
                        emit("download-progress", JSObject().put("trackId", id).put("received", received)
                            .put("size", if (size > 0) size else JSONObject.NULL))
                    }
                }
            }
        }
        if (changed) save()
        return busy
    }

    private fun fail(id: String, entry: JSONObject, message: String) {
        entry.put("state", "failed").put("error", message)
        emit("download-error", JSObject().put("trackId", id).put("message", message))
    }

    private fun failure(reason: Int): String = when (reason) {
        in 400..599 -> "The hub answered $reason"
        DownloadManager.ERROR_INSUFFICIENT_SPACE -> "Not enough space on this phone"
        DownloadManager.ERROR_CANNOT_RESUME -> "The download broke off and can't resume"
        else -> "The download failed (reason $reason)"
    }

    /** The cover is small: fetched directly, off the main thread. A missing cover isn't a failure. */
    private fun fetchCover(trackId: String, url: String) {
        val bearer = token
        val target = File(art, safe(trackId))
        thread(name = "lull-cover") {
            runCatching {
                val conn = URL(url).openConnection() as HttpURLConnection
                conn.connectTimeout = 10_000
                conn.readTimeout = 20_000
                bearer?.let { conn.setRequestProperty("Authorization", "Bearer $it") }
                if (conn.responseCode == 200) conn.inputStream.use { input -> target.outputStream().use { input.copyTo(it) } }
                conn.disconnect()
            }
        }
    }

    private fun save() {
        val tmp = File(dir, "index.json.tmp")
        tmp.writeText(index.toString())
        tmp.renameTo(indexFile)
    }

    /** Track ids come from the hub; as a file name only safe characters stay. */
    private fun safe(id: String) = id.replace(Regex("[^A-Za-z0-9._-]"), "_")
}
