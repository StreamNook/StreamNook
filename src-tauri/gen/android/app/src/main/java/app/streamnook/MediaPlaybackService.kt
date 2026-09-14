package app.streamnook

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Build
import android.os.IBinder
import java.net.URL
import kotlin.concurrent.thread

/**
 * Keeps StreamNook's audio alive while the screen is off, and puts real
 * transport controls on the lock screen and in the notification shade.
 *
 * WHY A SERVICE AT ALL. Playback is a `<video>` inside the WebView, not a native
 * player. Two separate things were stopping it on lock:
 *   1. `WryActivity.onPause()` calls `mWebView.onPause()`, which suspends the
 *      WebView's rendering AND its media. MainActivity now undoes that while
 *      this service is running - the exact trick the PiP path already uses.
 *   2. Even with the WebView awake, a backgrounded process with no foreground
 *      service gets frozen by the OS. A `mediaPlayback` foreground service is
 *      the sanctioned way to say "I am audio, keep me running".
 *
 * WHY THE PLATFORM MediaSession AND NOT media3. media3's `MediaSession` is built
 * around a `Player` implementation, and our player is a DOM element in a WebView
 * - satisfying that interface would mean a large shim for no benefit. The
 * platform `MediaSession` (API 21+) is designed for exactly the "I own my own
 * playback engine" case, and it costs ZERO new gradle dependencies, which
 * matters here: this module pins WorkManager and Firebase for Kotlin 1.9.25
 * metadata compatibility, so every added dependency is a real risk.
 *
 * Transport commands travel back to the web layer through [commandSink], which
 * MainActivity owns. Same process, so no IPC.
 */
class MediaPlaybackService : Service() {

  companion object {
    const val CHANNEL_ID = "streamnook_playback"
    const val NOTIFICATION_ID = 0x5E10

    const val ACTION_START = "app.streamnook.media.START"
    const val ACTION_UPDATE = "app.streamnook.media.UPDATE"
    const val ACTION_STOP = "app.streamnook.media.STOP"
    /** A transport command from the notification, forwarded to the web layer. */
    const val ACTION_CMD = "app.streamnook.media.CMD"

    const val EXTRA_TITLE = "title"
    const val EXTRA_ARTIST = "artist"
    const val EXTRA_ART_URL = "artUrl"
    const val EXTRA_PLAYING = "playing"
    const val EXTRA_CMD = "cmd"

    /**
     * Where transport commands go. MainActivity installs this and forwards into
     * the WebView. Volatile because the session callback runs on a binder thread.
     *
     * Values: "play" | "pause" | "stop".
     */
    @Volatile
    var commandSink: ((String) -> Unit)? = null

    /** True while the service is foregrounded, so onPause knows to keep the WebView awake. */
    @Volatile
    var isRunning: Boolean = false
  }

  private var session: MediaSession? = null

  private var title: String = "StreamNook"
  private var artist: String = ""
  private var playing: Boolean = true
  private var art: Bitmap? = null
  private var artUrl: String? = null

  override fun onBind(intent: Intent?): IBinder? = null

  override fun onCreate() {
    super.onCreate()
    createChannel()
    session = MediaSession(this, "StreamNook").apply {
      setCallback(object : MediaSession.Callback() {
        override fun onPlay() { commandSink?.invoke("play") }
        override fun onPause() { commandSink?.invoke("pause") }
        override fun onStop() { commandSink?.invoke("stop") }
        // Headset play/pause buttons arrive as MEDIA_BUTTON and are dispatched to
        // onPlay/onPause by the framework, so there is nothing extra to wire.
      })
      isActive = true
    }
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    when (intent?.action) {
      ACTION_STOP -> {
        stopSelf()
        return START_NOT_STICKY
      }
      ACTION_CMD -> {
        // Forward and return. The web layer will call back with ACTION_UPDATE
        // once the element has actually changed state.
        intent.getStringExtra(EXTRA_CMD)?.let { commandSink?.invoke(it) }
        return START_NOT_STICKY
      }
      ACTION_START, ACTION_UPDATE -> {
        title = intent.getStringExtra(EXTRA_TITLE) ?: title
        artist = intent.getStringExtra(EXTRA_ARTIST) ?: artist
        playing = intent.getBooleanExtra(EXTRA_PLAYING, playing)
        val url = intent.getStringExtra(EXTRA_ART_URL)
        if (!url.isNullOrEmpty() && url != artUrl) {
          artUrl = url
          loadArt(url)
        }
        pushSession()
        goForeground()
        isRunning = true
      }
    }
    // START_NOT_STICKY: if the OS kills us the WebView is gone too, so there is
    // nothing meaningful to restart into. A recreated service with no player
    // would just post a dead notification.
    return START_NOT_STICKY
  }

  override fun onDestroy() {
    isRunning = false
    session?.isActive = false
    session?.release()
    session = null
    super.onDestroy()
  }

  // --- audio focus ---------------------------------------------------------

  // THIS SERVICE MUST NOT REQUEST AUDIO FOCUS. Read this before adding it back.
  //
  // An earlier version did, and it stopped playback dead after about ten
  // seconds. Chromium ALREADY requests focus for the <video> element - logcat
  // shows its `org.chromium.content.browser.AudioFocusDelegate` holding focus
  // under our own package name. A second AUDIOFOCUS_GAIN request from the same
  // app does not "share" focus: the framework hands it over, Chromium sees a
  // focus LOSS for the element it is playing, and pauses the video. We were
  // stealing focus from our own player.
  //
  // The evidence, if it ever needs re-deriving:
  //   MediaFocusControl: abandonAudioFocus() ... AudioFocusDelegate ... app.streamnook
  //   MediaFocusControl: abandonAudioFocus() ... app.streamnook.h@... app.streamnook
  //   MediaSessionService: ... playbackState=PAUSED(2)
  //
  // So focus is the WebView's job and ducking, phone calls and other apps taking
  // over are all handled there already. This service does exactly three things:
  // hold the process in the foreground, own the MediaSession, and render the
  // notification.

  // --- session + notification ----------------------------------------------

  private fun pushSession() {
    val s = session ?: return
    val meta = MediaMetadata.Builder()
      .putString(MediaMetadata.METADATA_KEY_TITLE, title)
      .putString(MediaMetadata.METADATA_KEY_ARTIST, artist)
      .putString(MediaMetadata.METADATA_KEY_DISPLAY_TITLE, title)
      .putString(MediaMetadata.METADATA_KEY_DISPLAY_SUBTITLE, artist)
    art?.let { meta.putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, it) }
    s.setMetadata(meta.build())

    // A live stream has no seekable timeline, so no seek actions and an unknown
    // position. Advertising SEEK_TO on a live edge produces a scrubber that
    // cannot do anything.
    val actions = PlaybackState.ACTION_PLAY or
      PlaybackState.ACTION_PAUSE or
      PlaybackState.ACTION_PLAY_PAUSE or
      PlaybackState.ACTION_STOP
    s.setPlaybackState(
      PlaybackState.Builder()
        .setActions(actions)
        .setState(
          if (playing) PlaybackState.STATE_PLAYING else PlaybackState.STATE_PAUSED,
          PlaybackState.PLAYBACK_POSITION_UNKNOWN,
          1.0f,
        )
        .build(),
    )
  }

  private fun goForeground() {
    val n = buildNotification()
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
      startForeground(NOTIFICATION_ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
    } else {
      startForeground(NOTIFICATION_ID, n)
    }
  }

  private fun buildNotification(): Notification {
    val contentIntent = PendingIntent.getActivity(
      this,
      0,
      Intent(this, MainActivity::class.java).apply {
        flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
      },
      PendingIntent.FLAG_IMMUTABLE,
    )

    // The button sends a COMMAND, it does not set state directly.
    //
    // State flows one way: button -> commandSink -> the web layer acts on the
    // <video> -> the web layer reports the new state back via ACTION_UPDATE.
    // An earlier draft had the button flip `playing` itself, which drew a paused
    // button over a stream that was still playing the moment the web layer
    // disagreed. The element is the source of truth; this is a remote control.
    fun action(icon: Int, label: String, cmd: String): Notification.Action {
      val pi = PendingIntent.getService(
        this,
        cmd.hashCode(),
        Intent(this, MediaPlaybackService::class.java).apply {
          this.action = ACTION_CMD
          putExtra(EXTRA_CMD, cmd)
        },
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
      )
      return Notification.Action.Builder(Icon_(icon), label, pi).build()
    }

    val b = Notification.Builder(this, CHANNEL_ID)
      .setSmallIcon(R.drawable.ic_stat_notify)
      .setContentTitle(title)
      .setContentText(artist)
      .setContentIntent(contentIntent)
      .setOngoing(playing)
      .setVisibility(Notification.VISIBILITY_PUBLIC)
      .setOnlyAlertOnce(true)
    art?.let { b.setLargeIcon(it) }

    // The transport button. Play and pause are the same slot, swapped by state.
    val toggle = if (playing) {
      action(android.R.drawable.ic_media_pause, "Pause", "pause")
    } else {
      action(android.R.drawable.ic_media_play, "Play", "play")
    }
    b.addAction(toggle)

    session?.sessionToken?.let { token ->
      b.style = Notification.MediaStyle()
        .setMediaSession(token)
        .setShowActionsInCompactView(0)
    }
    return b.build()
  }

  /** Small shim so the icon call site stays readable. */
  @Suppress("FunctionName")
  private fun Icon_(res: Int) = android.graphics.drawable.Icon.createWithResource(this, res)

  /**
   * Channel art, fetched off the main thread.
   *
   * Best-effort by design: a missing avatar should cost the notification its
   * thumbnail, never the playback session. `BitmapFactory` returns null on
   * anything it cannot decode, and the whole thing is wrapped anyway.
   */
  private fun loadArt(url: String) {
    thread(isDaemon = true) {
      val bmp = try {
        URL(url).openStream().use { BitmapFactory.decodeStream(it) }
      } catch (_: Exception) {
        null
      }
      if (bmp != null) {
        art = bmp
        // Re-push on the main thread; the session and notification manager both
        // want it and we are on a worker here.
        android.os.Handler(mainLooper).post {
          if (isRunning) {
            pushSession()
            val nm = getSystemService(Context.NOTIFICATION_SERVICE) as? NotificationManager
            nm?.notify(NOTIFICATION_ID, buildNotification())
          }
        }
      }
    }
  }

  private fun createChannel() {
    val nm = getSystemService(Context.NOTIFICATION_SERVICE) as? NotificationManager ?: return
    if (nm.getNotificationChannel(CHANNEL_ID) != null) return
    // IMPORTANCE_LOW: a media transport notification must never make a sound or
    // vibrate. It is a control surface, not an alert.
    val ch = NotificationChannel(CHANNEL_ID, "Playback", NotificationManager.IMPORTANCE_LOW).apply {
      description = "Media controls for the stream you are listening to"
      setShowBadge(false)
      enableVibration(false)
      setSound(null, null)
    }
    nm.createNotificationChannel(ch)
  }
}
