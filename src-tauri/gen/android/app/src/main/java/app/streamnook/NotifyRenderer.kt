package app.streamnook

import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.os.SystemClock
import android.util.Log
import androidx.core.app.NotificationCompat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.ByteArrayOutputStream
import java.net.HttpURLConnection
import java.net.URL

/**
 * The one place a live/badge notification is turned into pixels, shared by the
 * WorkManager poll and the push receiver so the two lanes cannot drift apart in
 * look, grouping, or (critically) notification ids: both hash the same
 * `channelId` string, so whichever lane posts second replaces rather than
 * stacks.
 *
 * Not the Tauri notification plugin, deliberately: that plugin resolves icon
 * fields as drawable resource names, so it cannot show a streamer's avatar.
 */
object NotifyRenderer {
  const val TAG = "SNNotify"
  const val EXTRA_CHANNEL = "sn_channel"

  data class Item(
    /** Identity: hashed into the notification id. Prefixed for non-channels. */
    val channelId: String,
    /** Twitch login to open on tap. Empty for anything that is not a channel. */
    val login: String,
    val title: String,
    val body: String,
    val avatar: String?,
    /** OS category to post under. */
    val channel: String,
  )

  suspend fun postAll(context: Context, items: List<Item>) {
    val nm = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
    // Created defensively: both callers can run in a process where the web
    // shell has never booted, so neither can assume the channels exist.
    NotifyChannels.ensure(nm)

    for (item in items) {
      val intent = Intent(context, MainActivity::class.java).apply {
        flags = Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP
        // Only channels are openable. A badge has no login, so its tap just
        // brings the app up rather than trying to watch something.
        if (item.login.isNotEmpty()) putExtra(EXTRA_CHANNEL, item.login)
      }
      // The request code must vary per item too. Sharing one would make every
      // notification reuse the first intent, so they would all open the same
      // stream.
      val pending = PendingIntent.getActivity(
        context,
        item.channelId.hashCode(),
        intent,
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
      )

      val builder = NotificationCompat.Builder(context, item.channel)
        .setSmallIcon(R.drawable.ic_stat_notify)
        .setContentTitle(item.title)
        .setContentText(item.body)
        .setAutoCancel(true)
        // Grouped per category, so a badge drop never collapses under "channels
        // going live" and vice versa.
        .setGroup(groupFor(item.channel))
        .setContentIntent(pending)
      loadBitmap(item.avatar, twitchOnly = item.channel == NotifyChannels.LIVE)?.let { builder.setLargeIcon(it) }

      // Keyed on the item, so a second notification for the same channel or
      // badge replaces the first rather than stacking a duplicate.
      nm.notify(item.channelId.hashCode(), builder.build())
    }

    // One summary per category that actually produced something, so several
    // arriving at once collapse into a single row instead of filling the shade.
    for (channel in items.map { it.channel }.distinct()) {
      val summary = NotificationCompat.Builder(context, channel)
        .setSmallIcon(R.drawable.ic_stat_notify)
        .setGroup(groupFor(channel))
        .setGroupSummary(true)
        .setAutoCancel(true)
        .build()
      nm.notify(groupFor(channel).hashCode(), summary)
    }
  }

  private fun groupFor(channel: String) = "app.streamnook.GROUP.$channel"

  /**
   * Best effort: a missing avatar costs a thumbnail, never the notification.
   * Every fetch is bounded (5 s to connect, 5 s per read and 5 s for the whole
   * body, 1 MB, HTTP 200 with an image type). A
   * go-live picture (`twitchOnly`) also follows the iPhone's rules: HTTPS from
   * Twitch's picture host only, with no redirect followed.
   */
  private suspend fun loadBitmap(url: String?, twitchOnly: Boolean): Bitmap? {
    if (url.isNullOrEmpty()) return null
    val target = if (twitchOnly) AvatarRules.allowed(url) else AvatarRules.parse(url)
    if (target == null) {
      Log.w(TAG, "avatar skipped: not a fetchable picture URL")
      return null
    }
    return withContext(Dispatchers.IO) {
      var conn: HttpURLConnection? = null
      try {
        conn = (target.openConnection() as HttpURLConnection).apply {
          connectTimeout = AvatarRules.TIMEOUT_MS
          readTimeout = AvatarRules.TIMEOUT_MS
          instanceFollowRedirects = !twitchOnly
          useCaches = false
        }
        val deadline = SystemClock.elapsedRealtime() + AvatarRules.TIMEOUT_MS
        if (conn.responseCode != HttpURLConnection.HTTP_OK) return@withContext null
        if (conn.contentType?.lowercase()?.startsWith("image/") != true) return@withContext null
        if (conn.contentLengthLong > AvatarRules.MAX_BYTES) return@withContext null
        val bytes = ByteArrayOutputStream()
        conn.inputStream.use { input ->
          val buf = ByteArray(16 * 1024)
          while (true) {
            val n = input.read(buf)
            if (n < 0) break
            bytes.write(buf, 0, n)
            if (bytes.size() > AvatarRules.MAX_BYTES) return@withContext null
            if (SystemClock.elapsedRealtime() > deadline) return@withContext null
          }
        }
        // Null when the bytes are not an image: the notification goes out without one.
        BitmapFactory.decodeByteArray(bytes.toByteArray(), 0, bytes.size())
      } catch (e: Exception) {
        Log.w(TAG, "avatar fetch failed", e)
        null
      } finally {
        conn?.disconnect()
      }
    }
  }
}

/**
 * Which pictures a notification fetches, the same rules as the iPhone's go-live
 * banner extension (`ios/Sources/SNPush/LiveAvatar.swift`): HTTPS from
 * `static-cdn.jtvnw.net` only (no other port, no user info), 5 s timeouts, 1 MB.
 */
internal object AvatarRules {
  const val HOST = "static-cdn.jtvnw.net"
  const val TIMEOUT_MS = 5_000
  const val MAX_BYTES = 1_048_576

  /** Any http(s) URL, for pictures that are not go-live avatars (badges). */
  fun parse(raw: String): URL? = try {
    URL(raw).takeIf { it.protocol.equals("https", true) || it.protocol.equals("http", true) }
  } catch (e: Exception) {
    null
  }

  /** The URL when it is HTTPS on Twitch's picture host, else null. */
  fun allowed(raw: String): URL? {
    if (raw.length > 2048) return null
    val url = parse(raw) ?: return null
    if (!url.protocol.equals("https", true)) return null
    if (!url.host.equals(HOST, true)) return null
    if (url.userInfo != null) return null
    if (url.port != -1 && url.port != 443) return null
    return url
  }
}
