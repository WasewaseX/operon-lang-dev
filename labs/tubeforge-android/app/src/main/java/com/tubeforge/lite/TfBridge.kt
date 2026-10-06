package com.tubeforge.lite

import android.app.Activity
import android.app.DownloadManager
import android.content.Context
import android.database.Cursor
import android.net.Uri
import android.os.Environment
import android.webkit.JavascriptInterface
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/**
 * The JS bridge the WebView UI talks to. Everything Operon-shaped goes
 * through [OperonBridge] (the embedded core); everything download-shaped
 * goes through the platform DownloadManager. The gate mirrors the desktop
 * TubeForge policy keys one-for-one: the policy program computes the
 * rules inside Operon's deny-by-default sandbox, this class enforces them.
 */
class TfBridge(private val activity: Activity) {

    private fun filesDir(): File = activity.filesDir

    /** Copy bundled policy assets to the private dir once; return the path. */
    private fun policyPath(name: String): String {
        val dir = File(filesDir(), "policies").apply { mkdirs() }
        val out = File(dir, name)
        if (!out.exists()) {
            activity.assets.open("policies/$name").use { input ->
                out.outputStream().use { output -> input.copyTo(output) }
            }
        }
        return out.absolutePath
    }

    /** Run a policy program in the embedded Operon core; return its JSON result. */
    @JavascriptInterface
    fun runPolicy(name: String): String {
        val safe = name.filter { it.isLetterOrDigit() || it == '-' || it == '_' || it == '.' }
        return OperonBridge.runPolicyFile(policyPath(safe))
    }

    /**
     * The download gate — the exact desktop key set. Returns
     * {"allow":true} or {"allow":false,"reason":"..."} (+ optional
     * "maxBytes" / "quality" rewrites the UI applies before enqueueing).
     */
    @JavascriptInterface
    fun gate(policyJson: String, url: String, kind: String, bytes: Long, quality: Int): String {
        val p = try { JSONObject(policyJson) } catch (e: Exception) {
            return "{\"allow\":false,\"reason\":\"policy result unreadable\"}"
        }
        val allowKey = "allow_" + when (kind) {
            "video" -> "video"
            "audio" -> "audio"
            "playlist" -> "playlist"
            else -> "video"
        }
        if (p.optInt(allowKey, 1) == 0) {
            val reason = p.optString("reason_" + kind, "downloads of this kind are disabled by policy")
            return JSONObject().put("allow", false).put("reason", reason).toString()
        }
        for (dom in p.optJSONArray("deny_domains") ?: JSONArray()) {
            if (url.contains(dom.toString(), ignoreCase = true)) {
                return JSONObject().put("allow", false)
                    .put("reason", p.optString("reason_domain", "domain denied by policy")).toString()
            }
        }
        val allowList = p.optJSONArray("allow_domains")
        if (allowList != null && allowList.length() > 0) {
            var ok = false
            for (dom in allowList) if (url.contains(dom.toString(), ignoreCase = true)) ok = true
            if (!ok) return JSONObject().put("allow", false)
                .put("reason", p.optString("reason_domain", "domain not on the policy allowlist")).toString()
        }
        val out = JSONObject().put("allow", true)
        if (p.has("max_bytes")) out.put("maxBytes", p.optLong("max_bytes", Long.MAX_VALUE))
        if (p.has("max_quality")) out.put("qualityCap", p.optInt("max_quality", Int.MAX_VALUE))
        if (bytes > 0 && out.optLong("maxBytes", Long.MAX_VALUE) in 1 until bytes) {
            return JSONObject().put("allow", false)
                .put("reason", p.optString("reason_bytes", "transfer exceeds the policy size cap")).toString()
        }
        return out.toString()
    }

    /** Enqueue a download with the platform DownloadManager (app-private dir). */
    @JavascriptInterface
    fun startDownload(url: String, filename: String): String {
        return try {
            val safe = filename.replace(Regex("[^A-Za-z0-9._ -]"), "_").ifBlank { "download.bin" }
            val req = DownloadManager.Request(Uri.parse(url))
                .setTitle(safe)
                .setDescription("TubeForge Lite")
                .setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE_NOTIFY_COMPLETED)
                .setDestinationInExternalFilesDir(activity, Environment.DIRECTORY_DOWNLOADS, safe)
            val dm = activity.getSystemService(Context.DOWNLOAD_SERVICE) as DownloadManager
            JSONObject().put("allow", true).put("id", dm.enqueue(req)).toString()
        } catch (e: Exception) {
            JSONObject().put("allow", false).put("reason", e.message ?: "enqueue failed").toString()
        }
    }

    /** Query the platform queue: id, status, progress, bytes. */
    @JavascriptInterface
    fun listDownloads(): String {
        val dm = activity.getSystemService(Context.DOWNLOAD_SERVICE) as DownloadManager
        val q = DownloadManager.Query()
        val cur: Cursor = dm.query(q)
        val arr = JSONArray()
        val statusNames = mapOf(
            DownloadManager.STATUS_PENDING to "queued",
            DownloadManager.STATUS_RUNNING to "downloading",
            DownloadManager.STATUS_PAUSED to "paused",
            DownloadManager.STATUS_SUCCESSFUL to "done",
            DownloadManager.STATUS_FAILED to "failed",
        )
        while (cur.moveToNext()) {
            val id = cur.getLong(cur.getColumnIndexOrThrow(DownloadManager.COLUMN_ID))
            val st = cur.getInt(cur.getColumnIndexOrThrow(DownloadManager.COLUMN_STATUS))
            val done = cur.getLong(cur.getColumnIndexOrThrow(DownloadManager.COLUMN_BYTES_DOWNLOADED_SO_FAR))
            val total = cur.getLong(cur.getColumnIndexOrThrow(DownloadManager.COLUMN_TOTAL_SIZE_BYTES))
            val title = cur.getString(cur.getColumnIndexOrThrow(DownloadManager.COLUMN_TITLE)) ?: ""
            arr.put(
                JSONObject()
                    .put("id", id)
                    .put("title", title)
                    .put("status", statusNames[st] ?: "unknown")
                    .put("bytes", done)
                    .put("total", total)
            )
        }
        cur.close()
        return arr.toString()
    }

    @JavascriptInterface
    fun cancelDownload(id: Long): Boolean {
        val dm = activity.getSystemService(Context.DOWNLOAD_SERVICE) as DownloadManager
        return dm.remove(id) > 0
    }

    @JavascriptInterface
    fun downloadsPath(): String {
        val d = activity.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS)
        return d?.absolutePath ?: (filesDir().absolutePath + "/Download")
    }
}
