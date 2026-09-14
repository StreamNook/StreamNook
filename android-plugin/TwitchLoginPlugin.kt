package com.streamnook.dev

import android.annotation.SuppressLint
import android.app.Activity
import android.graphics.Color
import android.net.Uri
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.FrameLayout
import android.widget.TextView
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature

@InvokeArg
class OpenLoginArgs {
    lateinit var url: String
}

// Twitch's login page gates on browser version via User-Agent Client Hints
// (navigator.userAgentData), which report the real WebView engine version and an
// "Android WebView" brand — so a UA-string override alone still trips the
// "unsupported browser" wall on an older system WebView (e.g. the emulator's 124).
// This script runs BEFORE any page script and makes userAgentData report a current
// Chrome. On a real device with an up-to-date WebView it's a harmless no-op.
private const val UA_CLIENT_HINTS_SPOOF = """
(function(){
  try {
    var brands = [
      {brand:"Chromium", version:"140"},
      {brand:"Google Chrome", version:"140"},
      {brand:"Not/A)Brand", version:"24"}
    ];
    var full = [
      {brand:"Chromium", version:"140.0.0.0"},
      {brand:"Google Chrome", version:"140.0.0.0"},
      {brand:"Not/A)Brand", version:"24.0.0.0"}
    ];
    var uaData = {
      brands: brands, mobile: true, platform: "Android",
      getHighEntropyValues: function(h){
        return Promise.resolve({
          architecture:"", bitness:"", brands: brands, fullVersionList: full,
          mobile:true, model:"Pixel 6", platform:"Android", platformVersion:"14.0.0",
          uaFullVersion:"140.0.0.0", wow64:false
        });
      },
      toJSON: function(){ return {brands:brands, mobile:true, platform:"Android"}; }
    };
    Object.defineProperty(navigator, "userAgentData", { get: function(){ return uaData; }, configurable: true });
  } catch(e){}
})();
"""

// In-app Twitch login: shows Twitch's own login page in a native WebView overlay
// (no external browser), then the auth-token session cookie is read back from the
// app-global CookieManager. Mirrors the desktop embedded-webview harvest.
@TauriPlugin
class TwitchLoginPlugin(private val activity: Activity) : Plugin(activity) {
    private var overlay: FrameLayout? = null
    private var webView: WebView? = null

    // A recent mobile-Chrome UA (paired with the userAgentData spoof above) so
    // Twitch's version gate sees a current browser.
    private val chromeUa =
        "Mozilla/5.0 (Linux; Android 14; Pixel 6) AppleWebKit/537.36 " +
            "(KHTML, like Gecko) Chrome/140.0.0.0 Mobile Safari/537.36"

    @SuppressLint("SetJavaScriptEnabled")
    @Command
    fun openLogin(invoke: Invoke) {
        val args = invoke.parseArgs(OpenLoginArgs::class.java)
        activity.runOnUiThread {
            if (overlay != null) {
                // Already open — just navigate to the (possibly new) url.
                webView?.loadUrl(args.url)
                invoke.resolve()
                return@runOnUiThread
            }

            val root = FrameLayout(activity).apply {
                setBackgroundColor(Color.parseColor("#0E0E10"))
                layoutParams = ViewGroup.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.MATCH_PARENT
                )
            }

            val wv = WebView(activity).apply {
                layoutParams = FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.MATCH_PARENT
                ).apply { topMargin = dp(48) }
                settings.javaScriptEnabled = true
                settings.domStorageEnabled = true
                settings.userAgentString = chromeUa
                settings.databaseEnabled = true
            }

            CookieManager.getInstance().setAcceptCookie(true)
            CookieManager.getInstance().setAcceptThirdPartyCookies(wv, true)

            // Report a current Chrome via Client Hints before Twitch's browser-gate
            // script runs, so an old system WebView isn't flagged "unsupported".
            if (WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)) {
                WebViewCompat.addDocumentStartJavaScript(
                    wv,
                    UA_CLIENT_HINTS_SPOOF,
                    setOf("https://twitch.tv", "https://*.twitch.tv")
                )
            }

            wv.webViewClient = object : WebViewClient() {
                override fun shouldOverrideUrlLoading(
                    view: WebView,
                    request: WebResourceRequest
                ): Boolean {
                    return false // keep every navigation inside this WebView
                }
            }

            // Minimal top bar: a close affordance + "Sign in to Twitch" label. Thin,
            // borderless — no chunky chrome.
            val bar = FrameLayout(activity).apply {
                setBackgroundColor(Color.parseColor("#18181B"))
                layoutParams = FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, dp(48)
                )
            }
            val title = TextView(activity).apply {
                text = "Sign in to Twitch"
                setTextColor(Color.parseColor("#EFEFF1"))
                textSize = 15f
                layoutParams = FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT
                ).apply { gravity = Gravity.CENTER }
            }
            val close = TextView(activity).apply {
                text = "✕"
                setTextColor(Color.parseColor("#EFEFF1"))
                textSize = 18f
                setPadding(dp(16), 0, dp(20), 0)
                layoutParams = FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                    ViewGroup.LayoutParams.MATCH_PARENT
                ).apply { gravity = Gravity.CENTER_VERTICAL or Gravity.START }
                setOnClickListener { dismiss(); trigger("twitch-login-cancelled", JSObject()) }
            }
            bar.addView(title)
            bar.addView(close)

            root.addView(wv)
            root.addView(bar)

            activity.addContentView(
                root,
                ViewGroup.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.MATCH_PARENT
                )
            )

            overlay = root
            webView = wv
            wv.loadUrl(args.url)
            invoke.resolve()
        }
    }

    @Command
    fun getCookies(invoke: Invoke) {
        CookieManager.getInstance().flush()
        val cookies = CookieManager.getInstance().getCookie("https://www.twitch.tv") ?: ""
        val ret = JSObject()
        ret.put("cookies", cookies)
        invoke.resolve(ret)
    }

    @Command
    fun closeLogin(invoke: Invoke) {
        activity.runOnUiThread {
            dismiss()
            invoke.resolve()
        }
    }

    private fun dismiss() {
        overlay?.let { o ->
            (o.parent as? ViewGroup)?.removeView(o)
        }
        webView?.destroy()
        overlay = null
        webView = null
    }

    private fun dp(v: Int): Int =
        (v * activity.resources.displayMetrics.density).toInt()
}
