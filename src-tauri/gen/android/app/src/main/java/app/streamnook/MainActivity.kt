package app.streamnook

import android.Manifest
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.PictureInPictureParams
import android.app.RemoteAction
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.graphics.Color
import android.graphics.Rect
import android.graphics.drawable.Icon
import android.hardware.display.DisplayManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.PowerManager
import android.provider.Settings
import android.util.Rational
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import androidx.window.layout.FoldingFeature
import androidx.window.layout.WindowInfoTracker
import kotlinx.coroutines.launch
import org.json.JSONObject

class MainActivity : TauriActivity() {
  private var webView: WebView? = null

  // Last known insets in CSS px, readable synchronously from JS via the
  // `SNInsets` bridge (covers boot and page reloads, when the pushed CSS vars
  // have not been applied to the fresh document yet).
  @Volatile private var insetsJson: String = "{}"
  private var displayListener: DisplayManager.DisplayListener? = null
  private var screenReceiver: BroadcastReceiver? = null

  // Whether a stream is playing, so leaving the app enters system
  // picture-in-picture. Pushed from JS.
  @Volatile private var pipEligible: Boolean = false

  // Whether the system bars should be drawn with LIGHT content (a white clock),
  // which is what a dark theme background under them needs. Pushed from JS on
  // every palette change and re-asserted anywhere the window can lose the flag.
  // Seeded true because StreamNook boots dark.
  @Volatile private var barsLightContent: Boolean = true

  // Mirrors the <video> mute state, so the PiP window's action shows the right
  // icon and label. Pushed from JS on every mute change, from either control.
  @Volatile private var pipMuted: Boolean = false

  // The video's rect in window coordinates. Without it the OS crops and scales
  // the WHOLE activity into the PiP window, so the frames before the web layer
  // repaints show whatever full-screen panel happened to be open.
  @Volatile private var pipSourceRect: Rect? = null

  // Set when the PiP window was DISMISSED rather than expanded. Read and cleared
  // by the web shell on its way back to the foreground.
  @Volatile private var pipClosedPending: Boolean = false

  // Channel login from a tapped notification, waiting for the web shell to pick
  // it up. Needed for the cold case: the tap starts the activity, and there is
  // no WebView to hand it to for some time afterwards.
  @Volatile private var pendingChannel: String? = null

  // Fired by the PiP window's mute action. Private to the app; the receiver is
  // registered in code and explicitly NOT exported.
  private val muteReceiver = object : BroadcastReceiver() {
    override fun onReceive(context: Context?, intent: Intent?) {
      webView?.evaluateJavascript(
        "window.dispatchEvent(new CustomEvent('sn:pip-mute'))",
        null,
      )
    }
  }

  /**
   * Tell the web shell that the in-app login overlay was closed by the user.
   *
   * Without this, closing it leaves whatever was waiting on a token polling
   * against a screen that is no longer on top, with no way back: the sign-in
   * button stays busy forever and the setup wizard stays stuck mid-step.
   *
   * Rides the same `sn:` custom-event channel as the PiP signals. The plugin's
   * own `trigger` goes to Tauri's plugin event channel, which needs an ACL
   * grant an app-local plugin does not get, and working around that is the
   * reason its commands are forwarded through Rust in the first place.
   */
  fun notifyLoginCancelled() {
    webView?.post {
      webView?.evaluateJavascript(
        "window.dispatchEvent(new CustomEvent('sn:login-cancelled'))",
        null,
      )
    }
  }

  /**
   * Hands the shell a value read out of the login WebView's own storage.
   *
   * 7TV's sign-in finishes by writing a token into its page's localStorage
   * rather than by redirecting somewhere readable, so the value comes through
   * here instead of being pulled off a URL. Carried as JSON so the token cannot
   * break out of the string it is being embedded in.
   */
  fun notifyLoginStorage(key: String, value: String) {
    val detail = org.json.JSONObject().put("key", key).put("value", value)
    webView?.post {
      webView?.evaluateJavascript(
        "window.dispatchEvent(new CustomEvent('sn:login-storage',{detail:$detail}))",
        null,
      )
    }
  }

  /** The single source of PictureInPictureParams, used by every entry point and
   *  by every in-place update. */
  private fun buildPipParams(): PictureInPictureParams {
    val b = PictureInPictureParams.Builder().setAspectRatio(Rational(16, 9))
    pipSourceRect?.let { b.setSourceRectHint(it) }

    // Web content cannot draw usable controls inside a PiP window: taps there go
    // to the OS chrome. Close and expand are OS-provided; mute has to be ours.
    val iconRes = if (pipMuted) R.drawable.ic_pip_unmute else R.drawable.ic_pip_mute
    val label = if (pipMuted) "Unmute" else "Mute"
    val pending = PendingIntent.getBroadcast(
      this,
      0,
      Intent(ACTION_PIP_MUTE).setPackage(packageName),
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )
    b.setActions(
      listOf(RemoteAction(Icon.createWithResource(this, iconRes), label, label, pending)),
    )

    // Android 12+ enters PiP itself on the way out, which is the only reliable
    // path for a gesture-nav home swipe: onUserLeaveHint is not guaranteed to be
    // delivered there. onUserLeaveHint below stays as the pre-31 fallback.
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
      b.setAutoEnterEnabled(pipEligible)
    }
    return b.build()
  }

  /** Push the current params without entering PiP. No-ops harmlessly if the
   *  activity is in a state that will not accept them. */
  private fun refreshPipParams() {
    runOnUiThread {
      try {
        setPictureInPictureParams(buildPipParams())
      } catch (_: Exception) {
        /* PiP unavailable on this device/config */
      }
    }
  }

  /**
   * Push the wanted bar icon colour onto the window.
   *
   * Note the inversion: the platform flag is named for the BACKGROUND it is
   * compensating for, so `isAppearanceLightStatusBars = true` means a LIGHT bar
   * background and therefore DARK icons. Reading it as "light icons" is what
   * produced the black clock this exists to fix.
   *
   * Must be called on the UI thread.
   */
  private fun applyBarAppearance() {
    val controller = WindowCompat.getInsetsController(window, window.decorView)
    controller.isAppearanceLightStatusBars = !barsLightContent
    controller.isAppearanceLightNavigationBars = !barsLightContent
  }

  inner class InsetsBridge {
    @JavascriptInterface
    fun get(): String = insetsJson

    /**
     * Short edge of the ACTIVE display mode, in real pixels.
     *
     * This is the tallest video rendition the screen can actually show, and the
     * resolver uses it to stop resolving a 1440p source onto a screen that
     * cannot display it.
     *
     * Read from `display.mode`, NOT from DisplayMetrics or the WebView, because
     * those report the mode Android is CURRENTLY driving and this phone ships a
     * 1440x3168 panel that runs at 1080x2376 unless the user says otherwise.
     * Both numbers are real and they are not interchangeable: pick the wrong one
     * and you either cap a QHD display down to FHD, or fail to cap at all.
     *
     * `mode` is API 23+. minSdk is 26, so there is no fallback branch to write.
     */
    /**
     * Start or update the lock-screen media session.
     *
     * Called when a stream starts and on every play/pause, because the WEB LAYER
     * is the source of truth for `playing` - the notification button only sends
     * a command and waits to be told what happened.
     */
    @JavascriptInterface
    fun mediaSessionStart(title: String, artist: String, artUrl: String, playing: Boolean) {
      val i = Intent(this@MainActivity, MediaPlaybackService::class.java).apply {
        action = if (MediaPlaybackService.isRunning) {
          MediaPlaybackService.ACTION_UPDATE
        } else {
          MediaPlaybackService.ACTION_START
        }
        putExtra(MediaPlaybackService.EXTRA_TITLE, title)
        putExtra(MediaPlaybackService.EXTRA_ARTIST, artist)
        putExtra(MediaPlaybackService.EXTRA_ART_URL, artUrl)
        putExtra(MediaPlaybackService.EXTRA_PLAYING, playing)
      }
      // startForegroundService, not startService: from Android O a background
      // start of a plain service is illegal, and this can legitimately be called
      // while the activity is already pausing.
      ContextCompat.startForegroundService(this@MainActivity, i)
    }

    /** Tear the session down when playback ends or the stream is closed. */
    @JavascriptInterface
    fun mediaSessionStop() {
      if (!MediaPlaybackService.isRunning) return
      val i = Intent(this@MainActivity, MediaPlaybackService::class.java).apply {
        action = MediaPlaybackService.ACTION_STOP
      }
      // Plain startService: STOP does not foreground anything, and calling
      // startForegroundService without a matching startForeground would ANR.
      try {
        startService(i)
      } catch (_: IllegalStateException) {
        /* already gone */
      }
    }

    @JavascriptInterface
    fun displayShortEdge(): Int {
      val mode = (if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) display else windowManager.defaultDisplay)?.mode
        ?: return 0
      return minOf(mode.physicalWidth, mode.physicalHeight)
    }

    /**
     * Icon colour for the system bars, chosen by the active StreamNook theme.
     *
     * `light` means light CONTENT, i.e. the background under the bars is dark.
     * The app draws behind the bars, so this is not something the platform can
     * work out: its own guess comes from the phone's night-mode setting, which
     * says nothing about which theme is loaded.
     */
    @JavascriptInterface
    fun setBarsLightContent(light: Boolean) {
      barsLightContent = light
      runOnUiThread { applyBarAppearance() }
    }

    /** Hide/show the system bars (immersive playback, e.g. landscape watch). */
    @JavascriptInterface
    fun setImmersive(immersive: Boolean) {
      runOnUiThread {
        val controller = WindowCompat.getInsetsController(window, window.decorView)
        if (immersive) {
          // Only meaningful while the bars are hidden: it is what lets a swipe
          // bring them back temporarily instead of permanently.
          controller.systemBarsBehavior =
            WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
          controller.hide(WindowInsetsCompat.Type.systemBars())
        } else {
          controller.show(WindowInsetsCompat.Type.systemBars())
          // Reset it. This used to be assigned on BOTH paths, which left the
          // transient behaviour latched on the window for the life of the
          // process: one landscape stream, and every later show of the bars
          // could still come back as a scrim-backed transient overlay rather
          // than bars laid out against our content.
          controller.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_DEFAULT
          // Re-assert the icon colour: showing the bars can drop the appearance
          // flags, and nothing else re-runs enableEdgeToEdge (see onCreate).
          applyBarAppearance()
        }
      }
    }

    /** Keep the screen awake while a stream plays. */
    @JavascriptInterface
    fun setKeepScreenOn(on: Boolean) {
      runOnUiThread { webView?.keepScreenOn = on }
    }

    /** Mark whether leaving the app should enter system picture-in-picture.
     *  Pushes params too, since that is what carries autoEnterEnabled. */
    @JavascriptInterface
    fun setPipEligible(eligible: Boolean) {
      pipEligible = eligible
      refreshPipParams()
    }

    /**
     * Whether we are in the PiP window, readable SYNCHRONOUSLY.
     *
     * The `sn:pip` event and the `dataset.snPip` mirror both ride an async
     * evaluateJavascript, which races the WebView's own visibilitychange. The
     * lifecycle handler has to tell real backgrounding from PiP and must not
     * depend on winning that race, so it reads the activity directly.
     */
    @JavascriptInterface
    fun isInPip(): Boolean = isInPictureInPictureMode

    /** True once if the PiP window was closed rather than expanded. Reading it
     *  clears it, so the stream is only torn down for the dismissal it belongs
     *  to and not again on every later resume. */
    @JavascriptInterface
    fun consumePipClosed(): Boolean {
      val was = pipClosedPending
      pipClosedPending = false
      return was
    }

    /** The video's rect in DEVICE pixels, relative to the WebView. */
    @JavascriptInterface
    fun setPipSourceRect(l: Int, t: Int, r: Int, b: Int) {
      runOnUiThread {
        val off = IntArray(2)
        webView?.getLocationInWindow(off)
        pipSourceRect = Rect(l + off[0], t + off[1], r + off[0], b + off[1])
        try {
          setPictureInPictureParams(buildPipParams())
        } catch (_: Exception) {
          /* PiP unavailable on this device/config */
        }
      }
    }

    /** Flip the PiP mute action's icon to match the player. */
    @JavascriptInterface
    fun setPipMuted(muted: Boolean) {
      pipMuted = muted
      refreshPipParams()
    }

    /** Hand text (a stream link) to the system share sheet. */
    @JavascriptInterface
    fun share(text: String, subject: String) {
      runOnUiThread {
        try {
          val send = Intent(Intent.ACTION_SEND).apply {
            type = "text/plain"
            putExtra(Intent.EXTRA_TEXT, text)
            if (subject.isNotEmpty()) putExtra(Intent.EXTRA_SUBJECT, subject)
          }
          startActivity(Intent.createChooser(send, null))
        } catch (_: Exception) {
          /* no share target available */
        }
      }
    }

    /** Enter system picture-in-picture on demand.
     *
     *  No UI calls this any more, and that is deliberate: the activity IS the
     *  PiP window, so entering it from a button necessarily takes StreamNook off
     *  screen, which reads as the app closing. In-app shrinking is the web
     *  layer's mini player; system PiP is what LEAVING the app does. Kept for
     *  the bridge surface and for anything that genuinely wants to hand off. */
    @JavascriptInterface
    fun enterPip() {
      runOnUiThread {
        if (!isInPictureInPictureMode) {
          try {
            enterPictureInPictureMode(buildPipParams())
          } catch (_: Exception) {
            /* PiP unavailable on this device/config */
          }
        }
      }
    }

    // ---- Notifications ---------------------------------------------------
    //
    // The in-app permission prompt can reach a dead end: once Android decides
    // the user has permanently declined, requesting again returns denied
    // without showing anything. A category can also be silenced in system
    // settings while the app-level permission is still granted. Neither state
    // is visible to the plugin's is_permission_granted, so the panel needs to
    // read both and be able to open the one screen that can undo them.

    /** App-level switch, separate from the runtime permission. */
    @JavascriptInterface
    fun areNotificationsEnabled(): Boolean =
      NotificationManagerCompat.from(this@MainActivity).areNotificationsEnabled()

    /**
     * OS importance for a single channel: 0 is IMPORTANCE_NONE, meaning the
     * user silenced that category. -1 when the channel does not exist yet.
     */
    @JavascriptInterface
    fun channelImportance(id: String): Int {
      val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
      return nm.getNotificationChannel(id)?.importance ?: -1
    }

    /**
     * True while Android would still show a rationale, i.e. the user has
     * declined but not permanently. Once this is false and the permission is
     * still missing, an in-app request is a silent no-op and system settings is
     * the only way back.
     */
    @JavascriptInterface
    fun shouldShowNotificationRationale(): Boolean {
      if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return false
      return shouldShowRequestPermissionRationale(Manifest.permission.POST_NOTIFICATIONS)
    }

    /** The per-app notification screen: the only exit from a blocked state. */
    @JavascriptInterface
    fun openNotificationSettings() {
      runOnUiThread {
        try {
          startActivity(
            Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
              .putExtra(Settings.EXTRA_APP_PACKAGE, packageName)
          )
        } catch (_: Exception) {
          /* OEM without the standard screen */
        }
      }
    }

    /** Straight to one category, for fixing a single silenced channel. */
    @JavascriptInterface
    fun openChannelSettings(id: String) {
      runOnUiThread {
        try {
          startActivity(
            Intent(Settings.ACTION_CHANNEL_NOTIFICATION_SETTINGS)
              .putExtra(Settings.EXTRA_APP_PACKAGE, packageName)
              .putExtra(Settings.EXTRA_CHANNEL_ID, id)
          )
        } catch (_: Exception) {
          openNotificationSettings()
        }
      }
    }

    // ---- Background delivery ---------------------------------------------
    //
    // This is not a nicety. In the Rare and Restricted standby buckets Android
    // withholds network from jobs entirely, and Restricted is one batched run
    // per day, so an app the user opens infrequently simply stops delivering.
    // Being on the battery-optimisation allowlist exempts an app from Doze AND
    // from standby-bucket restrictions, so this toggle is what decides whether
    // background notifications work at all.

    @JavascriptInterface
    fun isIgnoringBatteryOptimizations(): Boolean {
      val pm = getSystemService(Context.POWER_SERVICE) as PowerManager
      return pm.isIgnoringBatteryOptimizations(packageName)
    }

    @JavascriptInterface
    fun requestIgnoreBatteryOptimizations() {
      runOnUiThread {
        try {
          startActivity(
            Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS)
              .setData(Uri.parse("package:$packageName"))
          )
        } catch (_: Exception) {
          // Some OEMs refuse the direct request. The list screen needs no
          // permission and always exists.
          try {
            startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))
          } catch (_: Exception) {
            /* nothing sensible left to open */
          }
        }
      }
    }

    @JavascriptInterface
    fun scheduleBackgroundChecks(intervalMinutes: Int) {
      NotifyScheduler.schedule(this@MainActivity, intervalMinutes.toLong())
    }

    @JavascriptInterface
    fun cancelBackgroundChecks() {
      NotifyScheduler.cancel(this@MainActivity)
    }

    /** One immediate poll, fired when the app returns to the foreground. */
    @JavascriptInterface
    fun runNotifyCheckNow() {
      NotifyScheduler.runOnce(this@MainActivity)
    }

    /**
     * Channel login from a tapped notification, or "" if there is none.
     *
     * Reading it clears it, the same contract as consumePipClosed. The web shell
     * drains this on mount, which is what covers the cold case: the tap launches
     * the activity and the WebView only exists a good while later, so pushing
     * would have nothing to push to.
     */
    @JavascriptInterface
    fun consumePendingChannel(): String {
      val channel = pendingChannel
      pendingChannel = null
      return channel ?: ""
    }
  }

  /**
   * A notification tap on a warm app arrives here rather than through onCreate.
   *
   * `super` first, and it is not optional: WryActivity forwards to
   * `Rust.onNewIntent` and TauriActivity forwards to `PluginManager`, which is
   * how the deep-link plugin sees `streamnook://` links. Skipping it would
   * silently break those.
   */
  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    val channel = intent.getStringExtra(NotifyWorker.EXTRA_CHANNEL) ?: return
    if (channel.isEmpty()) return
    pendingChannel = channel
    // The shell is already up in this path, so hand it over rather than waiting
    // for something to drain it. The bridge stays the fallback if no handler is
    // registered yet.
    runOnUiThread {
      webView?.evaluateJavascript(
        "window.__SN_OPEN_CHANNEL__ && window.__SN_OPEN_CHANNEL__(${JSONObject.quote(channel)})",
        null,
      )
    }
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    // Explicit styles, NOT the bare `enableEdgeToEdge()` default.
    //
    // The default is SystemBarStyle.auto, whose detector reads the PHONE's
    // night-mode setting and sets the bar icon colour from it. StreamNook's
    // chrome does not track the system setting - it tracks the chosen theme -
    // so on a phone in light mode that painted a black clock and a black
    // battery icon over StreamNook's dark background, unreadable. `dark` here
    // names the BACKGROUND being drawn behind the bars, so it means light icons.
    //
    // This is only the seed for the frames before a palette loads; the shell
    // corrects it per theme through the setBarsLightContent bridge. It matters
    // because nothing re-runs this: configChanges covers orientation, uiMode,
    // density and screen size, so the activity is never recreated.
    enableEdgeToEdge(
      statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
      navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
    )
    super.onCreate(savedInstanceState)

    // Cold start from a notification tap. Stashed for the shell to drain once
    // it mounts; there is no WebView to talk to at this point.
    intent?.getStringExtra(NotifyWorker.EXTRA_CHANNEL)?.takeIf { it.isNotEmpty() }?.let {
      pendingChannel = it
    }

    // Push token, fetched once per launch so `push_register` always has the
    // current one on disk (onNewToken only fires on rotation). Guarded: on a
    // device without Play services, or a build without google-services.json,
    // this throws and push simply stays dormant behind the poll lane.
    try {
      com.google.firebase.messaging.FirebaseMessaging.getInstance().token
        .addOnSuccessListener { token -> SNMessagingService.writeToken(this, token) }
    } catch (e: Throwable) {
      android.util.Log.i(NotifyRenderer.TAG, "push unavailable: ${e.message}")
    }

    // targetSdk is 36, so the export flag is mandatory. NOT_EXPORTED keeps the
    // mute intent reachable only by our own PendingIntent.
    ContextCompat.registerReceiver(
      this,
      muteReceiver,
      IntentFilter(ACTION_PIP_MUTE),
      ContextCompat.RECEIVER_NOT_EXPORTED,
    )

    // Android back: ask the web shell first (sheets, drill stacks, the watch
    // layer). Unconsumed presses background the task instead of killing the
    // session. TauriActivity sets handleBackNavigation=false, so without this
    // callback a back press would finish the activity.
    onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
      override fun handleOnBackPressed() {
        val wv = webView
        if (wv == null) {
          moveTaskToBack(true)
          return
        }
        wv.evaluateJavascript("window.__SN_BACK__ ? window.__SN_BACK__() : false") { result ->
          if (result != "true") moveTaskToBack(true)
        }
      }
    })
  }

  // Home/recents while a stream plays: keep it playing in a system PiP window.
  // On API 31+ autoEnterEnabled has usually already done this, hence the
  // isInPictureInPictureMode guard; this remains the path on 26..30.
  override fun onUserLeaveHint() {
    super.onUserLeaveHint()
    if (pipEligible && !isInPictureInPictureMode) {
      try {
        enterPictureInPictureMode(buildPipParams())
      } catch (_: Exception) {
        /* PiP unavailable on this device/config */
      }
    }
  }

  /**
   * Entering PiP moves the activity to PAUSED, and `WryActivity.onPause()`
   * pauses the WebView along with it. In PiP that is wrong: the window is still
   * on screen. A paused WebView stops drawing and stops media, so the PiP window
   * freezes on whatever was last rendered - the tab you happened to be on, not
   * the player - and playback dies with it.
   *
   * Undo it while pipped. `onPause` and `onPictureInPictureModeChanged` are not
   * ordered against each other across OEMs and API levels, so both sites do it;
   * `WebView.onResume()` is idempotent.
   */
  override fun onPause() {
    super.onPause()
    if (isInPictureInPictureMode) webView?.onResume()
    // Same trick, second reason. `WryActivity.onPause` calls `mWebView.onPause()`,
    // which suspends the WebView's media along with its rendering - that is
    // precisely why locking the phone killed the audio. While the media service
    // is foregrounded we undo it, so the <video> keeps decoding audio with the
    // screen off. `WebView.onResume()` is idempotent, so overlapping with the
    // PiP case above is harmless.
    if (MediaPlaybackService.isRunning) webView?.onResume()
  }

  override fun onPictureInPictureModeChanged(
    isInPictureInPictureMode: Boolean,
    newConfig: android.content.res.Configuration,
  ) {
    super.onPictureInPictureModeChanged(isInPictureInPictureMode, newConfig)
    // See onPause: keep the WebView live so the window keeps drawing and playing.
    if (isInPictureInPictureMode) webView?.onResume()
    // A PiP window has no system bars, so coming back out re-lays them out
    // without carrying the appearance flags over. Nothing else restores them.
    if (!isInPictureInPictureMode) applyBarAppearance()
    // Tell the web shell so it strips down to the bare player while pipped.
    val flag = if (isInPictureInPictureMode) "true" else "false"
    webView?.evaluateJavascript(
      "(function(){document.documentElement.dataset.snPip='" + flag +
        "';window.dispatchEvent(new CustomEvent('sn:pip',{detail:" + flag + "}));})()",
      null,
    )

    // Leaving PiP happens two ways and they mean opposite things:
    //   expand  -> the activity comes to the foreground (STARTED / RESUMED)
    //   close X -> the window is dismissed and the activity is on its way to
    //              stopped, so the lifecycle is still at CREATED here.
    // Dismissing the window is the viewer saying they are done with the stream.
    // Closing PiP does NOT finish the activity on this device (verified: the
    // process and the ActivityRecord both survive), so without this the stream
    // stays loaded and playing and reappears the next time the app is opened.
    if (!isInPictureInPictureMode && lifecycle.currentState == Lifecycle.State.CREATED) {
      // Belt: stop it now, so audio does not carry on in the background.
      webView?.evaluateJavascript(
        "window.dispatchEvent(new CustomEvent('sn:pip-closed'))",
        null,
      )
      // Braces: the activity is stopping, so that script may not get to run.
      // This flag is read synchronously on the way back in and cannot be lost.
      pipClosedPending = true
    }
  }

  /**
   * The activity is never recreated - configChanges covers orientation, uiMode,
   * density, locale and screen size - so a rotation, a fold, or the system
   * flipping to light mode arrives here instead of through a fresh onCreate.
   *
   * Re-assert the bar appearance, because the platform is free to re-resolve
   * the theme's own light/dark attributes on a uiMode change and ours is
   * derived from the StreamNook palette, not from the configuration.
   */
  override fun onConfigurationChanged(newConfig: android.content.res.Configuration) {
    super.onConfigurationChanged(newConfig)
    applyBarAppearance()
  }

  /**
   * Foldable posture -> CSS variables.
   *
   * A Z Fold unfolds with the hinge running VERTICALLY down the middle, so the
   * centre of the viewport is the one place content must not sit. Publishing the
   * fold's position lets the shell align a two-pane split TO the seam and keep
   * centred overlays off it.
   *
   * WindowManager rather than CSS on purpose: the Viewport Segments API
   * (`env(viewport-segment-*)`, `@media (horizontal-viewport-segments: 2)`) is
   * still experimental and origin-trial gated, so it cannot be relied on in a
   * WebView.
   */
  private fun observeFoldPosture(webView: WebView) {
    val density = resources.displayMetrics.density
    lifecycleScope.launch {
      lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
        WindowInfoTracker.getOrCreate(this@MainActivity)
          .windowLayoutInfo(this@MainActivity)
          .collect { layoutInfo ->
            val fold = layoutInfo.displayFeatures
              .filterIsInstance<FoldingFeature>()
              .firstOrNull()
            val js = if (fold == null) {
              "(function(){var d=document.documentElement,s=d.style;" +
                "d.dataset.snFold='none';" +
                "s.removeProperty('--sn-fold-x');s.removeProperty('--sn-fold-w');" +
                "window.dispatchEvent(new CustomEvent('sn:fold',{detail:null}));})()"
            } else {
              val b = fold.bounds
              fun dp(v: Int): Int = (v / density).toInt()
              // VERTICAL orientation means the hinge line itself runs top to
              // bottom, i.e. it splits the screen left/right. That is the Z Fold
              // book posture and the case a two-pane split should align to.
              val vertical = fold.orientation == FoldingFeature.Orientation.VERTICAL
              val posture = if (fold.state == FoldingFeature.State.HALF_OPENED) "half" else "flat"
              val json = JSONObject()
                .put("vertical", vertical)
                .put("posture", posture)
                .put("x", dp(b.left))
                .put("width", dp(b.width()))
                .put("y", dp(b.top))
                .put("height", dp(b.height()))
                .toString()
              "(function(){var d=document.documentElement,s=d.style;" +
                "d.dataset.snFold='" + (if (vertical) "vertical" else "horizontal") + "';" +
                "d.dataset.snFoldPosture='" + posture + "';" +
                "s.setProperty('--sn-fold-x','${dp(b.left)}px');" +
                "s.setProperty('--sn-fold-w','${dp(b.width())}px');" +
                "window.dispatchEvent(new CustomEvent('sn:fold',{detail:" + json + "}));})()"
            }
            webView.post { webView.evaluateJavascript(js, null) }
          }
      }
    }
  }

  override fun onWebViewCreate(webView: WebView) {
    this.webView = webView
    webView.addJavascriptInterface(InsetsBridge(), "SNInsets")
    observeFoldPosture(webView)
    observeDisplayMode(webView)
    observeScreenState(webView)

    // Transport controls (lock screen, shade, headset button, audio-focus loss)
    // arrive on a binder thread inside the service. Hand them to the web layer,
    // which owns the <video> and will report the resulting state back.
    MediaPlaybackService.commandSink = { cmd ->
      webView.post {
        webView.evaluateJavascript(
          "window.dispatchEvent(new CustomEvent('sn:media-cmd',{detail:'" + cmd + "'}))",
          null,
        )
      }
    }

    // Native inset bridge: Android WebView's env(safe-area-inset-*) is
    // unreliable under edge-to-edge (0 for the top without a display cutout,
    // 0 for the bottom with button navigation), so real WindowInsets are
    // pushed into CSS variables the mobile shell reads. IME height rides the
    // same channel for the chat input.
    val density = resources.displayMetrics.density
    ViewCompat.setOnApplyWindowInsetsListener(webView) { view, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
      )
      val ime = insets.getInsets(WindowInsetsCompat.Type.ime())
      // Gesture insets, which are NOT the system bars and are what the floating
      // mini player has to stay clear of.
      //
      // mandatorySystemGestures = the home / quick-switch strips (top and
      // bottom). An app CANNOT opt out of those the way it can opt out of back,
      // so the only workable answer is to keep draggable UI out of them -
      // otherwise a swipe meant for the system gets eaten by the mini player, or
      // the reverse.
      //
      // systemGestures additionally covers the left and right BACK strips. Those
      // an app may claim via setSystemGestureExclusionRects, but simply staying
      // clear of them is cheaper and cannot be silently ignored by the system.
      val gest = insets.getInsets(WindowInsetsCompat.Type.systemGestures())
      val mand = insets.getInsets(WindowInsetsCompat.Type.mandatorySystemGestures())
      fun px(v: Int): Int = (v / density).toInt()
      val kb = maxOf(0, px(ime.bottom) - px(bars.bottom))
      insetsJson = JSONObject()
        .put("top", px(bars.top))
        .put("right", px(bars.right))
        .put("bottom", px(bars.bottom))
        .put("left", px(bars.left))
        .put("kb", kb)
        .put("gestureLeft", px(gest.left))
        .put("gestureRight", px(gest.right))
        .put("gestureTop", px(mand.top))
        .put("gestureBottom", px(mand.bottom))
        .toString()
      val js = "(function(){var d=document.documentElement,s=d.style;" +
        "s.setProperty('--sn-inset-t','${px(bars.top)}px');" +
        "s.setProperty('--sn-inset-r','${px(bars.right)}px');" +
        "s.setProperty('--sn-inset-b','${px(bars.bottom)}px');" +
        "s.setProperty('--sn-kb','${kb}px');" +
        "s.setProperty('--sn-inset-l','${px(bars.left)}px');" +
        "s.setProperty('--sn-gesture-l','${px(gest.left)}px');" +
        "s.setProperty('--sn-gesture-r','${px(gest.right)}px');" +
        "s.setProperty('--sn-gesture-t','${px(mand.top)}px');" +
        "s.setProperty('--sn-gesture-b','${px(mand.bottom)}px');" +
        "d.dataset.snNativeInsets='true';})()"
      view.post { (view as WebView).evaluateJavascript(js, null) }
      insets
    }
    ViewCompat.requestApplyInsets(webView)
  }

  /**
   * Tell the WebView when the display MODE changes.
   *
   * This phone has a 1440x3168 panel that can run at either 1440x3168 or
   * 1080x2376, switchable from Settings and by the OS itself. The video resolver
   * caps quality to the screen's short edge, so a stale value is the difference
   * between a soft picture on a QHD screen and burning decode power on pixels an
   * FHD screen throws away.
   *
   * A `resize` in the WebView is NOT a usable signal here: both modes are 360
   * CSS px wide, so a resolution switch changes devicePixelRatio (3 -> 4) while
   * innerWidth/innerHeight stay put, and no resize fires. Hence a real
   * DisplayListener.
   */
  /**
   * Tell the web layer when the SCREEN goes off, as opposed to the app merely
   * being backgrounded. They are very different signals and only this one
   * requires action.
   *
   * Screen-off destroys the activity's window surface. Chromium then has a
   * WebContents with a video track and nowhere to render it, and tears the media
   * pipeline down - the audio player disappears from `dumpsys audio` entirely
   * rather than pausing. Dropping to an audio-only rendition is what survives
   * that, because there is no video left to render.
   *
   * ACTION_SCREEN_ON/OFF can only be registered at RUNTIME; a manifest receiver
   * is silently ignored for these, which is a well-known way to lose an hour.
   */
  private fun observeScreenState(webView: WebView) {
    screenReceiver = object : BroadcastReceiver() {
      override fun onReceive(context: Context?, intent: Intent?) {
        val event = when (intent?.action) {
          Intent.ACTION_SCREEN_OFF -> "sn:screen-off"
          Intent.ACTION_SCREEN_ON -> "sn:screen-on"
          else -> return
        }
        webView.post {
          webView.evaluateJavascript("window.dispatchEvent(new Event('$event'))", null)
        }
      }
    }
    val filter = IntentFilter().apply {
      addAction(Intent.ACTION_SCREEN_OFF)
      addAction(Intent.ACTION_SCREEN_ON)
    }
    // System broadcasts, so NOT_EXPORTED is both correct and required at
    // targetSdk 34+.
    ContextCompat.registerReceiver(this, screenReceiver, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
  }

  private fun observeDisplayMode(webView: WebView) {
    val dm = getSystemService(Context.DISPLAY_SERVICE) as? DisplayManager ?: return
    displayListener = object : DisplayManager.DisplayListener {
      override fun onDisplayAdded(displayId: Int) {}
      override fun onDisplayRemoved(displayId: Int) {}
      override fun onDisplayChanged(displayId: Int) {
        // Fires for rotation and refresh-rate changes too. Cheap and idempotent
        // on the JS side, so it is not worth filtering to mode changes only.
        webView.post {
          webView.evaluateJavascript(
            "window.dispatchEvent(new Event('sn:display-changed'))",
            null,
          )
        }
      }
    }
    dm.registerDisplayListener(displayListener, null)
  }

  override fun onDestroy() {
    try {
      unregisterReceiver(muteReceiver)
    } catch (_: IllegalArgumentException) {
      /* never registered */
    }
    displayListener?.let {
      (getSystemService(Context.DISPLAY_SERVICE) as? DisplayManager)?.unregisterDisplayListener(it)
    }
    displayListener = null
    screenReceiver?.let {
      try {
        unregisterReceiver(it)
      } catch (_: IllegalArgumentException) {
        /* never registered */
      }
    }
    screenReceiver = null
    // The sink captures this activity's WebView, so leaving it installed after
    // the activity dies would hold a destroyed view alive and post into it.
    MediaPlaybackService.commandSink = null
    super.onDestroy()
  }

  companion object {
    private const val ACTION_PIP_MUTE = "app.streamnook.PIP_MUTE"
  }
}
