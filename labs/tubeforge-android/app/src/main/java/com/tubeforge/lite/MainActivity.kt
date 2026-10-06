package com.tubeforge.lite

import android.app.Activity
import android.os.Bundle
import android.webkit.WebView
import android.webkit.WebViewClient

/**
 * TubeForge Lite for Android — the Operon-core build.
 *
 * One Activity, one WebView, two native halves:
 *   - liboperon.so: the full Operon language core (arm64), running policy
 *     programs in its deny-by-default sandbox — the same .op files, the
 *     same key set, the same fail-closed contract as the desktop builds.
 *   - the platform DownloadManager for the actual transfers.
 *
 * yt-dlp and ffmpeg are NOT bundled (the "lite" deal, same as desktop):
 * metadata extraction is the desktop lane's job; this build manages
 * direct media URLs under Operon-computed policy.
 */
class MainActivity : Activity() {
    private lateinit var web: WebView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        web = WebView(this)
        setContentView(web)
        web.settings.javaScriptEnabled = true
        web.settings.domStorageEnabled = true
        web.webViewClient = WebViewClient()
        web.addJavascriptInterface(TfBridge(this), "Android")
        web.loadUrl("file:///android_asset/ui.html")
    }

    override fun onBackPressed() {
        if (web.canGoBack()) web.goBack() else super.onBackPressed()
    }
}
