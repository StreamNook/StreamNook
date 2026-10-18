package app.streamnook

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.provider.Settings
import android.util.Log
import androidx.core.content.ContextCompat
import androidx.mediarouter.app.MediaRouteChooserDialog
import com.google.android.gms.cast.CastDevice
import com.google.android.gms.cast.HlsSegmentFormat
import com.google.android.gms.cast.HlsVideoSegmentFormat
import com.google.android.gms.cast.MediaInfo
import com.google.android.gms.cast.MediaLoadRequestData
import com.google.android.gms.cast.MediaMetadata
import com.google.android.gms.cast.MediaStatus
import com.google.android.gms.cast.framework.CastContext
import com.google.android.gms.cast.framework.CastSession
import com.google.android.gms.cast.framework.SessionManagerListener
import com.google.android.gms.cast.framework.media.RemoteMediaClient
import org.json.JSONObject

/**
 * The Google Cast device session: the one part of casting that has to live in
 * Kotlin, because only Google's SDK can hold a session.
 *
 * It does what it is told and reports what happens. WHAT plays on the TV, and
 * when it changes (a raid, a switch, an expired playlist), is decided in Rust
 * (`services/cast_session.rs`), which calls back in through [CastPlugin]. Every
 * session and player change goes to Rust through [CastBridge] as JSON.
 *
 * While a session is up it also holds MediaPlaybackService's card: the TV plays
 * the stream, but the process must stay unfrozen for Rust to keep deciding, and
 * the card's buttons are the remote.
 *
 * Main thread only: the SDK throws from anywhere else.
 */
object CastRemote {
  private const val TAG = "SNCast"
  private const val VOLUME_STEP = 0.05

  private var started = false
  private var appContext: Context? = null
  private var castContext: CastContext? = null
  private var client: RemoteMediaClient? = null
  private var lastMediaReport: String? = null

  /** True while a device session is connected. Read by the key handler. */
  @Volatile var isCasting: Boolean = false
    private set

  private var deviceName: String = ""
  private var cardTitle: String = ""
  private var cardSubtitle: String = ""
  private var playing: Boolean = false

  /** Bring the SDK up once per process. Idempotent. */
  fun init(context: Context) {
    if (started) return
    started = true
    val app = context.applicationContext
    appContext = app
    try {
      CastContext.getSharedInstance(app, ContextCompat.getMainExecutor(app))
        .addOnSuccessListener { ctx ->
          castContext = ctx
          ctx.sessionManager.addSessionManagerListener(listener, CastSession::class.java)
          report(JSONObject().put("kind", "available").put("value", true))
          // A session the SDK kept from an earlier launch.
          ctx.sessionManager.currentCastSession?.takeIf { it.isConnected }?.let { onConnected(it, "resumed") }
        }
        .addOnFailureListener { e ->
          // ModuleUnavailableException without Play services (microG included):
          // the page hides the cast option and offers mirroring only.
          Log.i(TAG, "Cast unavailable: $e")
          report(JSONObject().put("kind", "available").put("value", false))
        }
    } catch (e: Exception) {
      Log.i(TAG, "Cast unavailable: $e")
      report(JSONObject().put("kind", "available").put("value", false))
    }
  }

  // --- the session ----------------------------------------------------------

  private val listener = object : SessionManagerListener<CastSession> {
    override fun onSessionStarting(session: CastSession) = reportSession("starting", session)
    override fun onSessionStarted(session: CastSession, sessionId: String) = onConnected(session, "started")
    override fun onSessionStartFailed(session: CastSession, error: Int) = onDisconnected("failed", session)
    override fun onSessionEnding(session: CastSession) = reportSession("ending", session)
    override fun onSessionEnded(session: CastSession, error: Int) = onDisconnected("ended", session)
    override fun onSessionResuming(session: CastSession, sessionId: String) {}
    override fun onSessionResumed(session: CastSession, wasSuspended: Boolean) = onConnected(session, "resumed")
    override fun onSessionResumeFailed(session: CastSession, error: Int) = onDisconnected("ended", session)
    override fun onSessionSuspended(session: CastSession, reason: Int) = reportSession("suspended", session)
  }

  private fun onConnected(session: CastSession, state: String) {
    isCasting = true
    deviceName = session.castDevice?.friendlyName ?: ""
    client?.unregisterCallback(mediaCallback)
    client = session.remoteMediaClient?.also { it.registerCallback(mediaCallback) }
    lastMediaReport = null
    cardTitle = ""
    cardSubtitle = ""
    playing = false
    holdCard()
    reportSession(state, session)
    // A resumed session is already playing something; say what.
    if (state == "resumed") reportMedia()
  }

  private fun onDisconnected(state: String, session: CastSession) {
    isCasting = false
    client?.unregisterCallback(mediaCallback)
    client = null
    releaseCard()
    reportSession(state, session)
  }

  private fun reportSession(state: String, session: CastSession) {
    val device = session.castDevice
    report(
      JSONObject()
        .put("kind", "session")
        .put("state", state)
        .put("device", device?.friendlyName ?: "")
        // A speaker (Nest Audio, a speaker group) has no video out, and must
        // never be sent video: Google's audio guide says the app may fail.
        .put("video_out", device?.hasCapability(CastDevice.CAPABILITY_VIDEO_OUT) ?: true),
    )
  }

  // --- the receiver's player --------------------------------------------------

  private val mediaCallback = object : RemoteMediaClient.Callback() {
    override fun onStatusUpdated() = reportMedia()
  }

  private fun reportMedia() {
    val status = client?.mediaStatus
    val state = when (status?.playerState) {
      MediaStatus.PLAYER_STATE_PLAYING -> "playing"
      MediaStatus.PLAYER_STATE_PAUSED -> "paused"
      MediaStatus.PLAYER_STATE_BUFFERING -> "buffering"
      MediaStatus.PLAYER_STATE_LOADING -> "loading"
      else -> "idle"
    }
    val idleReason = when (status?.idleReason) {
      MediaStatus.IDLE_REASON_FINISHED -> "finished"
      MediaStatus.IDLE_REASON_CANCELED -> "cancelled"
      MediaStatus.IDLE_REASON_INTERRUPTED -> "interrupted"
      MediaStatus.IDLE_REASON_ERROR -> "error"
      else -> "none"
    }
    val json = JSONObject().put("kind", "media").put("state", state).put("idle_reason", idleReason)
    status?.mediaInfo?.customData?.optJSONObject("target")?.let { json.put("custom", it) }
    // The SDK repeats statuses (volume changes, queue updates); Rust only needs changes.
    val text = json.toString()
    if (text == lastMediaReport) return
    lastMediaReport = text
    report(json)

    val nowPlaying = state != "paused" && state != "idle"
    if (nowPlaying != playing) {
      playing = nowPlaying
      holdCard()
    }
  }

  /**
   * Put a stream on the receiver. `url` is the upstream media playlist, which
   * the receiver fetches itself (Twitch and Kick answer it with CORS `*`).
   */
  fun load(
    url: String,
    segmentFormat: String,
    audioOnly: Boolean,
    title: String,
    subtitle: String,
    customJson: String,
  ): Boolean {
    val c = client ?: return false
    val fmp4 = segmentFormat == "fmp4"
    val meta = MediaMetadata(
      if (audioOnly) MediaMetadata.MEDIA_TYPE_MUSIC_TRACK else MediaMetadata.MEDIA_TYPE_MOVIE,
    ).apply {
      putString(MediaMetadata.KEY_TITLE, title)
      putString(MediaMetadata.KEY_SUBTITLE, subtitle)
      if (audioOnly) putString(MediaMetadata.KEY_ARTIST, subtitle)
    }
    val custom = JSONObject().apply {
      try {
        put("target", JSONObject(customJson))
      } catch (_: Exception) {
        /* nothing to carry */
      }
    }
    // The segment formats are only read by the receiver's legacy player (MPL);
    // Shaka, the default HLS player since May 2026, sniffs the container.
    val info = MediaInfo.Builder(url)
      .setStreamType(MediaInfo.STREAM_TYPE_LIVE)
      .setContentType("application/x-mpegURL")
      .setMetadata(meta)
      .setCustomData(custom)
      .apply {
        if (audioOnly) {
          setHlsSegmentFormat(if (fmp4) HlsSegmentFormat.FMP4 else HlsSegmentFormat.TS_AAC)
        } else {
          setHlsSegmentFormat(if (fmp4) HlsSegmentFormat.FMP4 else HlsSegmentFormat.TS)
          setHlsVideoSegmentFormat(if (fmp4) HlsVideoSegmentFormat.FMP4 else HlsVideoSegmentFormat.MPEG2_TS)
        }
      }
      .build()
    val request = MediaLoadRequestData.Builder().setMediaInfo(info).setAutoplay(true).build()

    cardTitle = title
    cardSubtitle = subtitle
    holdCard()
    c.load(request).setResultCallback { result ->
      if (!result.status.isSuccess) {
        report(JSONObject().put("kind", "load_failed").put("code", result.status.statusCode))
      }
    }
    return true
  }

  /** "play" | "pause" | "stop". Stop ends the session (the receiver closes). */
  fun command(cmd: String): Boolean {
    if (!isCasting) return false
    when (cmd) {
      "play" -> client?.play()
      "pause" -> client?.pause()
      "stop" -> castContext?.sessionManager?.endCurrentSession(true)
      else -> return false
    }
    return true
  }

  fun stepVolume(up: Boolean) {
    val session = castContext?.sessionManager?.currentCastSession ?: return
    val next = (session.volume + if (up) VOLUME_STEP else -VOLUME_STEP).coerceIn(0.0, 1.0)
    try {
      session.volume = next
    } catch (e: Exception) {
      Log.w(TAG, "volume: $e")
    }
  }

  // --- pickers ----------------------------------------------------------------

  /** Google's device picker. False when the SDK never came up. */
  fun showPicker(activity: Activity): Boolean {
    val selector = castContext?.mergedSelector ?: return false
    MediaRouteChooserDialog(activity).apply { routeSelector = selector }.show()
    return true
  }

  /**
   * The system's screen-cast page. Mirroring is the system's job: apps cannot
   * drive Miracast, and the system's encoder beats anything an app could build.
   * OEMs differ, so a phone without the page answers false and the page shows
   * where to find it in quick settings instead.
   */
  fun openMirroring(activity: Activity): Boolean =
    try {
      activity.startActivity(Intent(Settings.ACTION_CAST_SETTINGS))
      true
    } catch (_: ActivityNotFoundException) {
      false
    } catch (_: SecurityException) {
      false
    }

  // --- the phone's card --------------------------------------------------------

  private fun holdCard() {
    val app = appContext ?: return
    if (!isCasting) return
    MediaPlaybackService.castWanted = true
    val on = if (deviceName.isEmpty()) "the TV" else deviceName
    val intent = Intent(app, MediaPlaybackService::class.java).apply {
      action = if (MediaPlaybackService.isRunning) {
        MediaPlaybackService.ACTION_UPDATE
      } else {
        MediaPlaybackService.ACTION_START
      }
      putExtra(MediaPlaybackService.EXTRA_TITLE, cardTitle.ifEmpty { "Casting to $on" })
      putExtra(
        MediaPlaybackService.EXTRA_ARTIST,
        if (cardSubtitle.isEmpty()) "StreamNook" else "$cardSubtitle, on $on",
      )
      putExtra(MediaPlaybackService.EXTRA_PLAYING, playing)
    }
    try {
      ContextCompat.startForegroundService(app, intent)
    } catch (e: Exception) {
      // ForegroundServiceStartNotAllowedException: a session the SDK resumed
      // while the app was in the background. The TV plays on regardless; the
      // card appears the next time the app is opened and anything changes.
      Log.w(TAG, "cast card not shown: $e")
    }
  }

  private fun releaseCard() {
    val app = appContext ?: return
    if (!MediaPlaybackService.castWanted) return
    MediaPlaybackService.castWanted = false
    try {
      app.startService(
        Intent(app, MediaPlaybackService::class.java).apply { action = MediaPlaybackService.ACTION_STOP },
      )
    } catch (_: IllegalStateException) {
      /* already gone */
    }
  }

  private fun report(json: JSONObject) {
    try {
      CastBridge.onEvent(json.toString())
    } catch (e: Throwable) {
      Log.w(TAG, "event not delivered: $e")
    }
  }
}
