package app.streamnook

import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.Log
import androidx.core.app.NotificationCompat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
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
      loadBitmap(item.avatar)?.let { builder.setLargeIcon(it) }

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

  /** Best effort: a missing avatar costs a thumbnail, never the notification. */
  private suspend fun loadBitmap(url: String?): Bitmap? {
    if (url.isNullOrEmpty()) return null
    return withContext(Dispatchers.IO) {
      try {
        URL(url).openStream().use { BitmapFactory.decodeStream(it) }
      } catch (e: Exception) {
        Log.w(TAG, "avatar fetch failed", e)
        null
      }
    }
  }
}
