package dev.kraken.app

import android.annotation.SuppressLint
import android.app.Activity
import android.app.AlertDialog
import android.content.pm.ApplicationInfo
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.view.Gravity
import android.view.View
import android.view.WindowInsets
import android.view.WindowInsetsController
import android.view.WindowManager
import android.webkit.CookieManager
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.PopupMenu
import android.widget.TextView
import android.widget.Toast
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import org.json.JSONObject

/**
 * Opens the browser build of the game in a WebView and loads Kraken into it:
 * `kraken-boot.js` runs at document start in every frame, and the kraken-web
 * files are answered from the APK's assets under `/__kraken/` on the game's
 * own origin, so the page's content security policy lets them load.
 */
class MainActivity : Activity() {
    private lateinit var web: WebView

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)

        if (applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0) {
            // chrome://inspect on the desktop reaches the page and its console.
            WebView.setWebContentsDebuggingEnabled(true)
        }

        web = WebView(this).apply {
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            settings.mediaPlaybackRequiresUserGesture = false
            settings.useWideViewPort = true
            settings.loadWithOverviewMode = true
            webViewClient = KrakenClient()
        }
        CookieManager.getInstance().setAcceptThirdPartyCookies(web, true)

        val root = FrameLayout(this)
        root.addView(web)
        root.addView(menuButton(), FrameLayout.LayoutParams(dp(44), dp(44), Gravity.TOP or Gravity.START).apply {
            setMargins(dp(8), dp(8), 0, 0)
        })
        setContentView(root)
        immersive()

        if (WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)) {
            val boot = assets.open("kraken/kraken-boot.js").bufferedReader().use { it.readText() }
            WebViewCompat.addDocumentStartJavaScript(web, boot, setOf("*"))
        } else {
            Toast.makeText(this, R.string.no_document_start, Toast.LENGTH_LONG).show()
        }

        if (savedInstanceState != null) {
            web.restoreState(savedInstanceState)
        } else {
            web.loadUrl(gameUrl())
        }
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        web.saveState(outState)
    }

    override fun onResume() {
        super.onResume()
        web.onResume()
        immersive()
    }

    override fun onPause() {
        web.onPause()
        CookieManager.getInstance().flush()
        super.onPause()
    }

    @Deprecated("Activity back handling")
    override fun onBackPressed() {
        if (web.canGoBack()) web.goBack() else @Suppress("DEPRECATION") super.onBackPressed()
    }

    private fun gameUrl(): String =
        getPreferences(MODE_PRIVATE).getString(PREF_URL, null) ?: DEFAULT_URL

    private fun askGameUrl() {
        val input = EditText(this).apply {
            setText(gameUrl())
            setSingleLine()
        }
        AlertDialog.Builder(this)
            .setTitle(R.string.game_url)
            .setView(input)
            .setPositiveButton(R.string.save) { _, _ ->
                val url = input.text.toString().trim()
                if (url.isNotEmpty()) {
                    getPreferences(MODE_PRIVATE).edit().putString(PREF_URL, url).apply()
                    web.loadUrl(url)
                }
            }
            .setNegativeButton(R.string.cancel, null)
            .show()
    }

    private fun menuButton(): View = TextView(this).apply {
        text = "K"
        textSize = 18f
        gravity = Gravity.CENTER
        setTextColor(Color.WHITE)
        background = GradientDrawable().apply {
            shape = GradientDrawable.OVAL
            setColor(Color.argb(160, 20, 60, 90))
        }
        setOnClickListener { view ->
            PopupMenu(this@MainActivity, view).apply {
                menu.add(0, 1, 0, R.string.menu_start)
                menu.add(0, 2, 1, R.string.menu_pause)
                menu.add(0, 3, 2, R.string.menu_stop)
                menu.add(0, 4, 3, R.string.menu_probe)
                menu.add(0, 5, 4, R.string.menu_url)
                menu.add(0, 6, 5, R.string.menu_reload)
                setOnMenuItemClickListener { item ->
                    when (item.itemId) {
                        1 -> command("start")
                        2 -> command("pause")
                        3 -> command("stop")
                        4 -> command("probe")
                        5 -> askGameUrl()
                        6 -> web.reload()
                    }
                    true
                }
            }.show()
        }
    }

    /** Hands a command to `kraken-boot.js` in whichever frame runs the game. */
    private fun command(name: String) {
        val message = JSONObject().put("__kraken", name)
        web.evaluateJavascript("window.postMessage($message, '*')", null)
    }

    private fun immersive() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            window.insetsController?.let {
                it.hide(WindowInsets.Type.systemBars())
                it.systemBarsBehavior = WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            }
        } else {
            @Suppress("DEPRECATION")
            window.decorView.systemUiVisibility = View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY or
                View.SYSTEM_UI_FLAG_FULLSCREEN or View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
        }
    }

    private fun dp(value: Int): Int = (value * resources.displayMetrics.density).toInt()

    private inner class KrakenClient : WebViewClient() {
        override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse? {
            val path = request.url.path ?: return null
            if (!path.startsWith(ASSET_PREFIX)) return null
            val name = Uri.decode(path.removePrefix(ASSET_PREFIX))
            if (name.isEmpty() || name.contains("..")) return null
            return try {
                val mime = when {
                    name.endsWith(".wasm") -> "application/wasm"
                    name.endsWith(".js") -> "text/javascript"
                    else -> "application/octet-stream"
                }
                WebResourceResponse(mime, null, assets.open("kraken/$name"))
            } catch (_: Exception) {
                WebResourceResponse("text/plain", null, 404, "Not Found", emptyMap(), null)
            }
        }
    }

    private companion object {
        const val PREF_URL = "game_url"
        const val DEFAULT_URL = "https://www.seafight.com/"
        const val ASSET_PREFIX = "/__kraken/"
    }
}
