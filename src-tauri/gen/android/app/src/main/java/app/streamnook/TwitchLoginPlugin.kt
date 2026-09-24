package app.streamnook

import android.annotation.SuppressLint
import android.app.Activity
import android.graphics.Color
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
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
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import org.json.JSONArray
import org.json.JSONObject

// Height of the overlay's title bar. The WebView is pushed down by the same
// amount, so the two are read from here rather than written out twice.
private const val BAR_DP = 48

/** How often the login page's storage is checked, once watching. */
private const val STORAGE_POLL_MS = 700L

/** Matches the desktop capture window, after which sign-in is assumed given up on. */
private const val STORAGE_WATCH_TIMEOUT_MS = 5 * 60 * 1000L

/** A session token is long. Anything shorter is a placeholder the page wrote early. */
private const val MIN_TOKEN_LEN = 50

@InvokeArg
class OpenLoginArgs {
    lateinit var url: String

    /**
     * Optional localStorage key to watch for inside the login WebView.
     *
     * This exists for 7TV, whose sign-in does not end at a redirect carrying a
     * token: it round-trips through Twitch and lands back on 7tv.app, which
     * writes the session token into ITS OWN localStorage. There is nothing in
     * the URL to intercept, so the only way to get it out is to read the
     * storage of the page we are showing.
     *
     * Desktop solves the same problem by injecting a script that stuffs the
     * token into an `about:blank#...` fragment and polling the window URL. That
     * dance exists because a Tauri window offers no better channel; a WebView we
     * own does, so the value is read directly and handed to the shell.
     */
    var watchStorageKey: String? = null

    /** Bar label. Defaults to Twitch, since that is what this is usually for. */
    var title: String? = null

    /**
     * Run the whole thing invisibly: no bar, nothing drawn, and a short deadline
     * after which it gives up on its own.
     *
     * For a silent session re-mint. 7TV's sign-in round-trips through Twitch
     * and, while the Twitch session in this WebView's cookie jar is still
     * alive, completes with no interaction and writes a fresh token into the
     * page's storage. Desktop does the same in a hidden window; this is the
     * phone's equivalent, and it is what stops the account reading as signed
     * out every 30 days.
     */
    var hidden: Boolean = false
}

@InvokeArg
class CookieUrlArgs {
    lateinit var url: String
}

@InvokeArg
class ExpireCookiesArgs {
    var urls: Array<String> = emptyArray()
}

/** A hidden re-mint either completes on its own quickly or it will not at all. */
private const val HIDDEN_WATCH_TIMEOUT_MS = 30 * 1000L

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

// Opens Kick's login form on its mobile site (see onPageFinished). The button
// is Kick's own `data-testid="login"`, present in the header on every page.
private const val KICK_OPEN_LOGIN = """
(function(){
  var n = 0;
  var t = setInterval(function(){
    if (document.querySelector('input[type=password]') || ++n > 20) { clearInterval(t); return; }
    var b = document.querySelector('button[data-testid="login"]');
    if (b) b.click();
  }, 500);
})();
"""

// In-app Twitch login: shows Twitch's own login page in a native WebView overlay
// (no external browser), then the auth-token session cookie is read back from the
// app-global CookieManager. Mirrors the desktop embedded-webview harvest.
@TauriPlugin
class TwitchLoginPlugin(private val activity: Activity) : Plugin(activity) {
    @Volatile
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
            // A silent re-mint never takes over a sign-in the person can see,
            // and a visible sign-in always wins over a re-mint in progress.
            if (overlay != null && args.hidden) {
                invoke.resolve()
                return@runOnUiThread
            }
            if (overlay != null && hiddenMode) {
                // No cancel event here. The page listens for one to end a
                // sign-in, and the visible sign-in taking over has ALREADY
                // registered its listener, so a cancel ended it before it
                // began. The re-mint being replaced hears the new sign-in's
                // token (same key) or runs out its own short deadline.
                dismiss()
            }
            hiddenMode = args.hidden
            // A redirect left from an abandoned attempt carries a stale state
            // and would fail the next exchange.
            kickRedirect = null
            openedUrl = args.url
            bouncedHome = false
            if (overlay != null) {
                // Already open — just navigate to the (possibly new) url. The
                // watch is restarted rather than left alone, since reusing the
                // overlay for a different sign-in means a different key.
                stopStorageWatch()
                webView?.loadUrl(args.url)
                args.watchStorageKey?.let { startStorageWatch(it) }
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
                ).apply { topMargin = dp(BAR_DP) }
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
                    setOf(
                        "https://twitch.tv", "https://*.twitch.tv",
                        "https://kick.com", "https://*.kick.com"
                    )
                )
            }

            wv.webViewClient = object : WebViewClient() {
                override fun shouldOverrideUrlLoading(
                    view: WebView,
                    request: WebResourceRequest
                ): Boolean {
                    // The drops grant and Kick consent are finished the moment
                    // the site redirects back with the credential; the page it
                    // would land on is never needed. Everything else stays
                    // inside this WebView.
                    val u = request.url.toString()
                    if (captureDropsRedirect(u) || captureKickRedirect(u)) return true
                    // An app-store or intent:// hop (a site's "Open in app")
                    // has nothing to load in here; swallowing it keeps the
                    // sign-in page instead of an unknown-scheme error page.
                    if (!u.startsWith("http://") && !u.startsWith("https://")) {
                        android.util.Log.i("SNLogin", "ignored non-web navigation: ${u.substringBefore(':')}")
                        return true
                    }
                    return false
                }

                /**
                 * Kick's mobile site ignores /login and shows its home page with
                 * the form closed, so a person asked to sign in has to hunt for
                 * the account icon. Press Kick's own Log In button for them.
                 * Retried briefly because the site hydrates after load, and it
                 * stops the moment a password field exists.
                 */
                override fun onPageFinished(view: WebView, url: String?) {
                    super.onPageFinished(view, url)
                    val u = url ?: return
                    if (hostPath(u) != "kick.com/login") return
                    view.evaluateJavascript(KICK_OPEN_LOGIN, null)
                }

                /**
                 * Dismiss as soon as authorization is DONE, instead of waiting
                 * for the token to arrive.
                 *
                 * The device-code flow polls id.twitch.tv on Twitch's advertised
                 * `interval` (typically 5s) and sleeps BEFORE each attempt, so
                 * after you approve, nothing collects the token until the next
                 * tick. Until then this overlay is still up showing twitch.tv -
                 * which is the "thrown into the Twitch website for a few
                 * seconds" that everyone notices. It is the poll gap, not a
                 * rendering delay, so no amount of UI tweaking fixes it.
                 *
                 * Where approval lands is not guessed - Twitch states it. The
                 * authorize URL carries the destination as a `redirect_uri`
                 * query param, so it is captured on the way through and matched
                 * on arrival. Observed flow:
                 *
                 *   www.twitch.tv/activate?device-code=XXXX
                 *   auth.twitch.tv/authorize?...&redirect_uri=<dest>&...
                 *   <dest>                                  <- approved, done
                 *
                 * Today `<dest>` is www.twitch.tv/settings/connections. An
                 * earlier version of this guessed the home page instead, which
                 * simply never matched and did nothing. Reading it off the
                 * authorize URL means a Twitch-side change fixes itself.
                 *
                 * Deliberately ADDITIVE. The normal close on
                 * `twitch-login-complete` still runs and `dismiss()` is
                 * idempotent, so the worst case is closing slightly early -
                 * which is the behaviour being asked for anyway.
                 */
                override fun onPageStarted(
                    view: WebView,
                    url: String?,
                    favicon: android.graphics.Bitmap?,
                ) {
                    super.onPageStarted(view, url, favicon)
                    val u = url ?: return
                    // Belt to the override above: a fragment-only landing can
                    // reach here without an override call.
                    if (captureDropsRedirect(u) || captureKickRedirect(u)) return
                    if (bounceHomeToActivate(view, u)) return
                    // Logged so the real post-approval URL stays visible in
                    // logcat if Twitch ever moves where it lands. Never the
                    // fragment: that is where a credential would ride.
                    android.util.Log.i("SNLogin", "nav: ${u.substringBefore('#')}")
                    rememberRedirectTarget(u)
                    // In watch mode the redirect is NOT the finish line. 7TV's
                    // sign-in also travels through auth.twitch.tv/authorize, so
                    // the redirect_uri captured above is 7TV's own callback, and
                    // dismissing there would tear the page down in the moment
                    // before it writes the token. The token is the signal.
                    if (storageWatchKey == null && isApprovalLanding(u)) {
                        android.util.Log.i("SNLogin", "authorized; dismissing overlay early")
                        activity.runOnUiThread { dismiss() }
                    }
                }
            }

            // Minimal top bar: a close affordance + "Sign in to Twitch" label. Thin,
            // borderless — no chunky chrome.
            val bar = FrameLayout(activity).apply {
                setBackgroundColor(Color.parseColor("#18181B"))
                layoutParams = FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, dp(BAR_DP)
                )
            }
            val title = TextView(activity).apply {
                text = args.title ?: "Sign in to Twitch"
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
                setOnClickListener {
                    dismiss()
                    trigger("twitch-login-cancelled", JSObject())
                    // The trigger above lands on the plugin event channel, which
                    // nothing can subscribe to without an ACL grant this plugin
                    // does not have. This is the one the shell actually hears.
                    (activity as? MainActivity)?.notifyLoginCancelled()
                }
            }
            bar.addView(title)
            bar.addView(close)

            root.addView(wv)
            if (args.hidden) {
                // Attached (a WebView needs a window to run its page) but never
                // drawn and never touchable, and the page fills the frame since
                // there is no bar to sit under.
                (wv.layoutParams as FrameLayout.LayoutParams).topMargin = 0
                root.visibility = View.INVISIBLE
                root.alpha = 0f
            } else {
                root.addView(bar)
            }

            activity.addContentView(
                root,
                ViewGroup.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.MATCH_PARENT
                )
            )

            // The activity draws edge to edge and lays out through the display
            // cutout, and a view handed to addContentView gets none of that
            // sorted out for it: it lands at the very top of the screen. Left
            // alone the bar sits exactly on the status bar, so the title runs
            // into the clock and the close button ends up under the camera
            // cutout, where it is genuinely hard to hit.
            //
            // The bar grows by the top inset and pads its own contents down by
            // the same amount, so it still reads as one strip that happens to
            // start behind the status bar rather than a floating band with a
            // gap above it. The rest keeps the page clear of the gesture bar
            // and, in landscape, of the cutout.
            ViewCompat.setOnApplyWindowInsetsListener(root) { view, insets ->
                val bars = insets.getInsets(
                    WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
                )
                bar.layoutParams = FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, dp(BAR_DP) + bars.top
                )
                bar.setPadding(0, bars.top, 0, 0)
                (wv.layoutParams as FrameLayout.LayoutParams).also {
                    it.topMargin = dp(BAR_DP) + bars.top
                    wv.layoutParams = it
                }
                view.setPadding(bars.left, 0, bars.right, bars.bottom)
                insets
            }
            // This view is added after the activity's first inset pass, so
            // nothing would dispatch to it without asking.
            ViewCompat.requestApplyInsets(root)

            overlay = root
            webView = wv
            wv.loadUrl(args.url)
            args.watchStorageKey?.let { startStorageWatch(it) }
            invoke.resolve()
        }
    }

    /**
     * Sign the embedded browser out too. The CookieManager is app-global and
     * outlives the app's own session: without this, signing out of StreamNook
     * left the previous person's Twitch web session in the jar, so the next
     * sign-in skipped Twitch's login page and the silent 7TV re-mint kept
     * minting for the account that had just left. Web storage goes with it,
     * since that is where 7TV keeps its token.
     */
    @Command
    fun clearCookies(invoke: Invoke) {
        activity.runOnUiThread {
            val cm = CookieManager.getInstance()
            cm.removeAllCookies {
                cm.flush()
                android.webkit.WebStorage.getInstance().deleteAllData()
                invoke.resolve()
            }
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

    /** Cookies for any origin. The Kick sign-in reads kick.com's HttpOnly
     *  session from here, which page script cannot see. */
    @Command
    fun getCookiesFor(invoke: Invoke) {
        val args = invoke.parseArgs(CookieUrlArgs::class.java)
        CookieManager.getInstance().flush()
        val cookies = CookieManager.getInstance().getCookie(args.url) ?: ""
        val ret = JSObject()
        ret.put("cookies", cookies)
        invoke.resolve(ret)
    }

    /** Whether the overlay is on screen. Rust polls it to notice a close.
     *  Read directly (the field is @Volatile) rather than hopping to the UI
     *  thread, so a busy UI thread never stalls the Rust poller. */
    @Command
    fun isOpen(invoke: Invoke) {
        val ret = JSObject()
        ret.put("open", overlay != null)
        invoke.resolve(ret)
    }

    /**
     * Sign one site out of the embedded browser without touching the others.
     * CookieManager has no per-site removal, so each cookie is overwritten as
     * expired, both host-only and on the parent domain, since Kick sets some
     * on `.kick.com`.
     */
    @Command
    fun expireCookies(invoke: Invoke) {
        val args = invoke.parseArgs(ExpireCookiesArgs::class.java)
        activity.runOnUiThread {
            val cm = CookieManager.getInstance()
            for (u in args.urls) {
                val raw = cm.getCookie(u) ?: continue
                val host = Uri.parse(u).host ?: continue
                val parent = "." + host.split('.').takeLast(2).joinToString(".")
                raw.split(';')
                    .map { it.substringBefore('=').trim() }
                    .filter { it.isNotEmpty() }
                    .forEach { name ->
                        cm.setCookie(u, "$name=; Max-Age=0; Path=/")
                        cm.setCookie(u, "$name=; Max-Age=0; Path=/; Domain=$parent")
                    }
            }
            cm.flush()
            invoke.resolve()
        }
    }

    @Command
    fun closeLogin(invoke: Invoke) {
        activity.runOnUiThread {
            dismiss()
            invoke.resolve()
        }
    }

    // ── Drops sign-in: the implicit grant ────────────────────────────────────
    // Twitch closed the device-code grant on the drops client, so the phone
    // signs in the way the desktop now does: an authorize url that redirects to
    // https://www.twitch.tv/#access_token=... once the account has approved
    // (no consent screen at all for an account that already has). The token is
    // taken off that redirect HERE and held natively until Rust asks for it, so
    // it never travels through the page as an event payload and never loads
    // as a url anyone could read off a bar or a log.

    @Volatile
    private var dropsToken: String? = null

    /** True when the url is the drops landing; the overlay closes and the shell
     *  is told to collect. */
    private fun captureDropsRedirect(url: String): Boolean {
        if (!url.startsWith("https://www.twitch.tv/")) return false
        val fragment = url.substringAfter('#', "")
        if (fragment.isEmpty()) return false
        val token = fragment.split('&').firstNotNullOfOrNull { pair ->
            val kv = pair.split('=', limit = 2)
            if (kv.size == 2 && kv[0] == "access_token" && kv[1].isNotEmpty()) kv[1] else null
        } ?: return false
        dropsToken = token
        android.util.Log.i("SNLogin", "drops grant landed; credential held for Rust")
        // Posted, not run inline: this is called from inside the WebView's own
        // navigation callback, and dismiss() destroys that WebView.
        storageWatchHandler.post {
            dismiss()
            (activity as? MainActivity)?.notifyDropsRedirect()
        }
        return true
    }

    /** Hand over the captured drops token once, or an empty string. */
    @Command
    fun takeDropsToken(invoke: Invoke) {
        val ret = JSObject()
        ret.put("token", dropsToken ?: "")
        dropsToken = null
        invoke.resolve(ret)
    }

    // ── Kick consent: the authorization-code redirect ───────────────────────
    // Kick registers exactly one redirect, http://localhost:3000/callback. On
    // the phone nothing listens there, so the navigation is taken here and
    // cancelled, and Rust collects the url (code + state) with takeKickRedirect.
    // The url is stored BEFORE the overlay closes; the Rust relay reads the
    // open flag first and the redirect second, and relies on that order.

    @Volatile
    private var kickRedirect: String? = null

    private fun captureKickRedirect(url: String): Boolean {
        if (!url.startsWith("http://localhost:3000/callback?")) return false
        kickRedirect = url
        android.util.Log.i("SNLogin", "kick consent landed; redirect held for Rust")
        // Posted for the same reason as the drops capture: dismiss() destroys
        // the WebView whose callback this is.
        storageWatchHandler.post { dismiss() }
        return true
    }

    /** Hand over the captured Kick redirect once, or an empty string. */
    @Command
    fun takeKickRedirect(invoke: Invoke) {
        val ret = JSObject()
        ret.put("url", kickRedirect ?: "")
        kickRedirect = null
        invoke.resolve(ret)
    }

    /** host+path of where approving will land, read off the authorize URL. */
    private var approvalLanding: String? = null

    /** The url this overlay was opened for, so a detour can be undone. */
    private var openedUrl: String? = null
    private var bouncedHome: Boolean = false

    /**
     * Twitch's mobile site does not always return to the activate page after a
     * sign-in: a fresh login can land on the home page instead, under an
     * "Open in App" sheet, with the device code nowhere in sight. Seen on a
     * user's phone: they closed the overlay from there, and the sign-in never
     * finished. When the overlay was opened for /activate and the page has
     * wandered to the bare home page, send it back to the activate url once.
     * The user is signed in by then, so it shows the code ready to approve.
     * Once only: a second arrival at home is left alone rather than looped.
     */
    private fun bounceHomeToActivate(view: WebView, url: String): Boolean {
        if (bouncedHome) return false
        val opened = openedUrl ?: return false
        if (!opened.contains("/activate")) return false
        if (hostPath(url) != "www.twitch.tv") return false
        bouncedHome = true
        android.util.Log.i("SNLogin", "landed on home mid sign-in; returning to the activate page")
        view.post { view.loadUrl(opened) }
        return true
    }

    /** Normalised host+path, so query strings and trailing slashes do not matter. */
    private fun hostPath(url: String): String? {
        return try {
            val u = android.net.Uri.parse(url)
            val host = u.host?.lowercase()
            if (host == null) null else host + u.path.orEmpty().trimEnd('/')
        } catch (_: Exception) {
            null
        }
    }

    /**
     * On the way through `auth.twitch.tv/authorize`, note the `redirect_uri` it
     * carries. That is where approving will send us, straight from Twitch,
     * rather than a hardcoded guess that silently rots.
     */
    private fun rememberRedirectTarget(url: String) {
        try {
            val u = android.net.Uri.parse(url)
            if (u.host?.lowercase() != "auth.twitch.tv") return
            val redirect = u.getQueryParameter("redirect_uri") ?: return
            approvalLanding = hostPath(redirect)
            android.util.Log.i("SNLogin", "approval will land on: $approvalLanding")
        } catch (_: Exception) {
            /* leave the fallback in place */
        }
    }

    /**
     * True once we reach the page approving redirects to.
     *
     * Falls back to the observed destination when the authorize step was not
     * seen (already-signed-in sessions can skip straight through), so this still
     * works without having captured the redirect first.
     */
    private fun isApprovalLanding(url: String): Boolean {
        val here = hostPath(url) ?: return false
        approvalLanding?.let { return here == it }
        return here == "www.twitch.tv/settings/connections"
    }

    // ── Watching the login page's own storage ────────────────────────────────
    // See OpenLoginArgs.watchStorageKey for why this exists.

    private var storageWatchKey: String? = null
    private val storageWatchHandler = Handler(Looper.getMainLooper())
    private var storageWatchTick: Runnable? = null
    /** Whether the current overlay is a hidden re-mint (see OpenLoginArgs.hidden). */
    private var hiddenMode: Boolean = false

    private fun startStorageWatch(key: String) {
        val js = "(function(){try{return window.localStorage.getItem(" +
            JSONObject.quote(key) + ")}catch(e){return null}})()"
        val timeout = if (hiddenMode) HIDDEN_WATCH_TIMEOUT_MS else STORAGE_WATCH_TIMEOUT_MS
        val deadline = SystemClock.elapsedRealtime() + timeout
        lateinit var tick: Runnable
        tick = Runnable {
            val wv = webView
            if (wv == null || storageWatchKey != key) return@Runnable
            if (SystemClock.elapsedRealtime() > deadline) {
                android.util.Log.i("SNLogin", "storage watch for $key timed out")
                if (hiddenMode) {
                    // Nobody can close an invisible overlay; it closes itself
                    // and tells the shell the re-mint did not happen.
                    dismiss()
                    (activity as? MainActivity)?.notifyLoginCancelled()
                }
                return@Runnable
            }
            wv.evaluateJavascript(js) { raw ->
                // Only meaningful once the flow has landed back on the origin
                // that owns the key, so a null here is the ordinary case for
                // most of the sign-in rather than a failure.
                val value = decodeJsString(raw)
                if (value != null && value.length > MIN_TOKEN_LEN && isExpiredJwt(value)) {
                    // The key's origin keeps its storage between sessions, so the
                    // token from last month is sitting there when the flow lands
                    // back on it, until the page writes the new one. Taking it
                    // closed the sign-in in under a second and handed the app an
                    // expired session: the Sign in button looked like it did
                    // nothing, and the silent monthly re-mint never re-minted.
                    android.util.Log.i("SNLogin", "ignoring an expired $key; waiting for the new one")
                    storageWatchHandler.postDelayed(tick, STORAGE_POLL_MS)
                } else if (value != null && value.length > MIN_TOKEN_LEN) {
                    android.util.Log.i("SNLogin", "captured $key (${value.length} chars)")
                    stopStorageWatch()
                    (activity as? MainActivity)?.notifyLoginStorage(key, value)
                    dismiss()
                } else {
                    storageWatchHandler.postDelayed(tick, STORAGE_POLL_MS)
                }
            }
        }
        storageWatchKey = key
        storageWatchTick = tick
        storageWatchHandler.postDelayed(tick, STORAGE_POLL_MS)
    }

    /** True for a JWT whose `exp` is past (or within a minute of it). Anything
     *  that is not a readable JWT is not judged here. */
    private fun isExpiredJwt(value: String): Boolean {
        val parts = value.split('.')
        if (parts.size != 3) return false
        return try {
            val payload = String(
                android.util.Base64.decode(
                    parts[1],
                    android.util.Base64.URL_SAFE or android.util.Base64.NO_PADDING or android.util.Base64.NO_WRAP,
                ),
                Charsets.UTF_8,
            )
            val exp = JSONObject(payload).optLong("exp", 0L)
            exp > 0L && exp * 1000L <= System.currentTimeMillis() + 60_000L
        } catch (_: Exception) {
            false
        }
    }

    private fun stopStorageWatch() {
        storageWatchTick?.let { storageWatchHandler.removeCallbacks(it) }
        storageWatchTick = null
        storageWatchKey = null
    }

    /** `evaluateJavascript` hands back JSON, so `null` and quoting are real. */
    private fun decodeJsString(raw: String?): String? {
        if (raw == null || !raw.startsWith("\"")) return null
        return try {
            JSONArray("[$raw]").getString(0)
        } catch (_: Exception) {
            null
        }
    }

    private fun dismiss() {
        stopStorageWatch()
        overlay?.let { o ->
            (o.parent as? ViewGroup)?.removeView(o)
        }
        webView?.destroy()
        overlay = null
        webView = null
        hiddenMode = false
    }

    private fun dp(v: Int): Int =
        (v * activity.resources.displayMetrics.density).toInt()
}
