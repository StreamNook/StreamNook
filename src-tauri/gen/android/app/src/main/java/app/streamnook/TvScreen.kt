package app.streamnook

import android.app.Activity
import android.app.Presentation
import android.content.Context
import android.graphics.Color
import android.hardware.display.DisplayManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.Display
import android.view.WindowManager
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MimeTypes
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.exoplayer.DefaultLoadControl
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.AspectRatioFrameLayout
import androidx.media3.ui.PlayerView
import org.json.JSONObject

/**
 * TV screen mode: the stream full-frame on a mirrored TV.
 *
 * Mirroring shows the TV a picture of the whole phone, so a 16:9 stream inside
 * a ~20:9 phone screen fills about 65% of a 16:9 TV. A wireless display that
 * carries FLAG_PRESENTATION (Miracast sinks such as a Roku do) can take a
 * Presentation: this app's own window on the TV, at the TV's size, while the
 * phone keeps showing the app. That window holds a native player.
 *
 * WHY A NATIVE PLAYER AND NOT A SECOND WEBVIEW. The page's player is replaced
 * by a card while the TV plays, so there is still one decoder either way, and a
 * Media3 player is far lighter than a second Chromium surface with a page to
 * keep in sync. It plays the phone's own relay playlist (127.0.0.1 is allowed
 * cleartext by network_security_config.xml), so ad filtering and quality are
 * exactly what the phone would play.
 *
 * Rust (`services/tv_screen.rs`) decides when to show and what to play; this
 * reports displays and player state back through [CastBridge].
 *
 * Main thread only.
 */
object TvScreen {
  private const val TAG = "SNTvScreen"

  private var activity: Activity? = null
  private var displays: DisplayManager? = null
  private var display: Display? = null
  private var lastDisplayReport: String? = null
  private var presentation: TvPresentation? = null
  private var player: ExoPlayer? = null
  private var lastPlayerReport: String? = null

  fun init(act: Activity) {
    activity = act
    val dm = act.getSystemService(Context.DISPLAY_SERVICE) as DisplayManager
    displays = dm
    dm.registerDisplayListener(listener, Handler(Looper.getMainLooper()))
    refresh()
  }

  /** The activity is going away: leave the TV to plain mirroring. */
  fun release() {
    displays?.unregisterDisplayListener(listener)
    displays = null
    hide()
    activity = null
    display = null
    lastDisplayReport = null
  }

  private val listener = object : DisplayManager.DisplayListener {
    override fun onDisplayAdded(displayId: Int) = refresh()
    override fun onDisplayRemoved(displayId: Int) = refresh()
    override fun onDisplayChanged(displayId: Int) = refresh()
  }

  /** The display the system offers for presentations, if any. */
  private fun pick(): Display? =
    displays?.getDisplays(DisplayManager.DISPLAY_CATEGORY_PRESENTATION)?.firstOrNull {
      it.displayId != Display.DEFAULT_DISPLAY &&
        (it.flags and Display.FLAG_PRESENTATION) != 0 &&
        it.state != Display.STATE_OFF
    }

  private fun refresh() {
    val d = pick()
    display = d
    // The presentation follows the display: a new one, or none at all.
    val shownOn = presentation?.display?.displayId
    if (shownOn != null && shownOn != d?.displayId) hide()
    val json = JSONObject().put("kind", "display").put("present", d != null)
    if (d != null) {
      json.put("name", d.name ?: "TV").put("width", d.mode.physicalWidth).put("height", d.mode.physicalHeight)
    }
    // onDisplayChanged fires for every state flicker; Rust only needs changes.
    val text = json.toString()
    if (text == lastDisplayReport) return
    lastDisplayReport = text
    report(json)
  }

  /** Play `url` full-frame on the presentation display. */
  fun show(url: String): Boolean {
    val act = activity ?: return false
    val d = display ?: return false
    if (presentation?.display?.displayId != d.displayId) {
      presentation?.dismiss()
      val p = TvPresentation(act, d)
      p.setOnDismissListener { if (presentation === p) onPresentationGone() }
      try {
        p.show()
      } catch (e: WindowManager.InvalidDisplayException) {
        Log.w(TAG, "presentation refused: $e")
        reportPlayer("idle", "The TV stopped accepting the picture.")
        return false
      }
      presentation = p
    }
    val exo = player ?: buildPlayer(act).also { player = it }
    presentation?.view?.player = exo
    exo.setMediaItem(
      MediaItem.Builder().setUri(url).setMimeType(MimeTypes.APPLICATION_M3U8).build(),
    )
    exo.playWhenReady = true
    exo.prepare()
    return true
  }

  /** Back to plain mirroring. */
  fun hide() {
    val p = presentation
    presentation = null
    p?.dismiss()
    releasePlayer()
  }

  fun control(action: String): Boolean {
    val exo = player ?: return false
    when (action) {
      "play" -> exo.play()
      "pause" -> exo.pause()
      else -> return false
    }
    return true
  }

  /** The system took the presentation away (display removed, task removed). */
  private fun onPresentationGone() {
    presentation = null
    releasePlayer()
    refresh()
  }

  private fun buildPlayer(ctx: Context): ExoPlayer {
    // A live stream needs little buffer, and every second of it is memory.
    val load = DefaultLoadControl.Builder().setBufferDurationsMs(8_000, 24_000, 1_500, 2_500).build()
    val audio = AudioAttributes.Builder().setUsage(C.USAGE_MEDIA).setContentType(C.AUDIO_CONTENT_TYPE_MOVIE).build()
    return ExoPlayer.Builder(ctx.applicationContext).setLoadControl(load).build().apply {
      // The page's own player is gone while the TV plays, so this one can hold
      // audio focus without stealing it from the WebView.
      setAudioAttributes(audio, true)
      addListener(object : Player.Listener {
        override fun onPlaybackStateChanged(state: Int) = reportState()
        override fun onIsPlayingChanged(isPlaying: Boolean) = reportState()
        override fun onPlayerError(error: PlaybackException) {
          if (error.errorCode == PlaybackException.ERROR_CODE_BEHIND_LIVE_WINDOW) {
            // Fell behind the live window: jump back to the live edge.
            seekToDefaultPosition()
            prepare()
            return
          }
          Log.w(TAG, "player error: ${error.errorCodeName}")
          reportPlayer("idle", "The TV couldn't play this stream.")
        }
      })
    }
  }

  private fun releasePlayer() {
    val exo = player ?: return
    player = null
    exo.release()
    reportPlayer("idle", null)
  }

  private fun reportState() {
    val exo = player ?: return
    val state = when {
      exo.isPlaying -> "playing"
      exo.playbackState == Player.STATE_BUFFERING -> "buffering"
      exo.playbackState == Player.STATE_READY -> "paused"
      exo.playbackState == Player.STATE_IDLE -> "loading"
      else -> "idle"
    }
    reportPlayer(state, null)
  }

  private fun reportPlayer(state: String, error: String?) {
    val json = JSONObject().put("kind", "tv_player").put("state", state)
    if (error != null) json.put("error", error)
    val text = json.toString()
    if (text == lastPlayerReport) return
    lastPlayerReport = text
    report(json)
  }

  private fun report(json: JSONObject) {
    try {
      CastBridge.onEvent(json.toString())
    } catch (e: Throwable) {
      Log.w(TAG, "event not delivered: $e")
    }
  }

  /** The window on the TV: black, with the player filling it, letterboxed to fit. */
  private class TvPresentation(ctx: Context, display: Display) : Presentation(ctx, display) {
    var view: PlayerView? = null

    override fun onCreate(savedInstanceState: Bundle?) {
      super.onCreate(savedInstanceState)
      val v = PlayerView(context).apply {
        useController = false
        resizeMode = AspectRatioFrameLayout.RESIZE_MODE_FIT
        setShutterBackgroundColor(Color.BLACK)
        setBackgroundColor(Color.BLACK)
        keepScreenOn = true
      }
      view = v
      setContentView(v)
    }
  }
}
