package app.streamnook

import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.graphics.BitmapFactory
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.Person
import androidx.core.app.RemoteInput
import androidx.core.graphics.drawable.IconCompat
import java.net.HttpURLConnection
import java.net.URL

/**
 * The whisper notification: one conversation per person in messaging style,
 * newest message last, with a Reply box. The core decided to show it
 * (services::whisper_inbox: the setting, streamer mode, not the conversation
 * on screen) and posts it through MentionNotifyPlugin.postWhisper, never the
 * page, because wry pauses the WebView whenever the app leaves the screen.
 *
 * A tap opens that conversation (MainActivity, EXTRA_WHISPER). A reply goes to
 * [WhisperReplyReceiver], which sends it through the core's own whisper send.
 */
object WhisperNotifier {
  const val EXTRA_WHISPER = "sn_whisper"
  const val KEY_REPLY = "sn_whisper_reply"
  private const val GROUP = "app.streamnook.GROUP.whispers"
  /** Messages kept in one conversation's notification. */
  private const val KEEP = 6
  /** Whisper ids already posted, so one delivered twice is posted once. */
  private val recent = ArrayDeque<String>()
  private const val RECENT_MAX = 200

  data class Item(
    /** The conversation's key in the core's inbox (the sender's Twitch id). */
    val key: String,
    val id: String,
    val userId: String,
    val login: String,
    val name: String,
    val avatar: String,
    val text: String,
    val atMs: Long,
  )

  @Synchronized
  private fun claim(id: String): Boolean {
    if (id.isEmpty()) return true
    if (recent.contains(id)) return false
    recent.addLast(id)
    while (recent.size > RECENT_MAX) recent.removeFirst()
    return true
  }

  private fun notifId(key: String) = "whisper:$key".hashCode()

  private fun me() = Person.Builder().setName("You").build()

  /** Best effort: a missing picture costs the face, never the notification. */
  private fun icon(url: String): IconCompat? {
    if (url.isEmpty()) return null
    return try {
      val conn = URL(url).openConnection() as HttpURLConnection
      conn.connectTimeout = 3000
      conn.readTimeout = 3000
      conn.inputStream.use { BitmapFactory.decodeStream(it) }?.let { IconCompat.createWithBitmap(it) }
    } catch (e: Exception) {
      Log.w(NotifyRenderer.TAG, "whisper avatar fetch failed", e)
      null
    }
  }

  private fun sender(item: Item): Person {
    val builder = Person.Builder()
      .setName(item.name.ifEmpty { item.login })
      .setKey("twitch:${item.userId.ifEmpty { item.login }}")
    icon(item.avatar)?.let { builder.setIcon(it) }
    return builder.build()
  }

  fun post(context: Context, item: Item) {
    if (item.key.isEmpty() || !claim(item.id)) return
    try {
      val nm = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
      NotifyChannels.ensure(nm)
      val style = existingStyle(nm, item.key) ?: NotificationCompat.MessagingStyle(me())
      style.setConversationTitle(null)
      style.setGroupConversation(false)
      style.addMessage(NotificationCompat.MessagingStyle.Message(item.text, item.atMs, sender(item)))
      trim(style)
      nm.notify(notifId(item.key), build(context, item, style, alert = true))
      summary(context, nm)
    } catch (e: Exception) {
      // A missing permission or a dead channel costs this one notification.
      Log.w(NotifyRenderer.TAG, "whisper notification failed", e)
    }
  }

  /** The reply went out: it joins the conversation as yours, which also ends
   *  the reply box's spinner. Quiet: you just wrote it. */
  fun replied(context: Context, item: Item, text: String) {
    update(context, item) { style ->
      style.addMessage(NotificationCompat.MessagingStyle.Message(text, System.currentTimeMillis(), null as Person?))
    }
  }

  /** The reply did not go out: the conversation comes back as it was (ending
   *  the spinner), and a second notification says why, in the core's words. */
  fun replyFailed(context: Context, item: Item, reason: String) {
    update(context, item) { }
    try {
      val nm = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
      val who = item.name.ifEmpty { item.login }
      val failed = NotificationCompat.Builder(context, NotifyChannels.WHISPERS)
        .setSmallIcon(R.drawable.ic_stat_notify)
        .setContentTitle("Your reply to $who was not sent")
        .setContentText(reason)
        .setStyle(NotificationCompat.BigTextStyle().bigText(reason))
        .setAutoCancel(true)
        .setContentIntent(openIntent(context, item))
        .build()
      nm.notify("whisper-failed:${item.key}".hashCode(), failed)
    } catch (e: Exception) {
      Log.w(NotifyRenderer.TAG, "whisper failure notice failed", e)
    }
  }

  private fun update(context: Context, item: Item, change: (NotificationCompat.MessagingStyle) -> Unit) {
    try {
      val nm = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
      val style = existingStyle(nm, item.key) ?: NotificationCompat.MessagingStyle(me())
      change(style)
      trim(style)
      nm.notify(notifId(item.key), build(context, item, style, alert = false))
    } catch (e: Exception) {
      Log.w(NotifyRenderer.TAG, "whisper notification update failed", e)
    }
  }

  private fun existingStyle(nm: NotificationManager, key: String): NotificationCompat.MessagingStyle? {
    val existing = nm.activeNotifications.firstOrNull { it.id == notifId(key) }?.notification ?: return null
    return NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(existing)
  }

  private fun trim(style: NotificationCompat.MessagingStyle) {
    val messages = style.messages
    while (messages.size > KEEP) messages.removeAt(0)
  }

  private fun openIntent(context: Context, item: Item): PendingIntent {
    val intent = Intent(context, MainActivity::class.java).apply {
      flags = Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP
      putExtra(EXTRA_WHISPER, item.key)
    }
    return PendingIntent.getActivity(
      context,
      notifId(item.key),
      intent,
      PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )
  }

  private fun build(
    context: Context,
    item: Item,
    style: NotificationCompat.MessagingStyle,
    alert: Boolean,
  ): android.app.Notification {
    val reply = Intent(context, WhisperReplyReceiver::class.java).apply {
      putExtra("key", item.key)
      putExtra("id", item.id)
      putExtra("user_id", item.userId)
      putExtra("login", item.login)
      putExtra("name", item.name)
      putExtra("avatar", item.avatar)
    }
    // MUTABLE: the system writes the typed text into this intent.
    val replyPending = PendingIntent.getBroadcast(
      context,
      notifId(item.key),
      reply,
      PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )
    val input = RemoteInput.Builder(KEY_REPLY).setLabel("Reply").build()
    val action = NotificationCompat.Action.Builder(R.drawable.ic_stat_notify, "Reply", replyPending)
      .addRemoteInput(input)
      .setAllowGeneratedReplies(false)
      .setSemanticAction(NotificationCompat.Action.SEMANTIC_ACTION_REPLY)
      .setShowsUserInterface(false)
      .build()
    return NotificationCompat.Builder(context, NotifyChannels.WHISPERS)
      .setSmallIcon(R.drawable.ic_stat_notify)
      .setStyle(style)
      .setCategory(NotificationCompat.CATEGORY_MESSAGE)
      .setShowWhen(true)
      .setAutoCancel(true)
      .setOnlyAlertOnce(!alert)
      .setSilent(!alert)
      .setGroup(GROUP)
      .setContentIntent(openIntent(context, item))
      .addAction(action)
      .build()
  }

  private fun summary(context: Context, nm: NotificationManager) {
    val summary = NotificationCompat.Builder(context, NotifyChannels.WHISPERS)
      .setSmallIcon(R.drawable.ic_stat_notify)
      .setGroup(GROUP)
      .setGroupSummary(true)
      .setAutoCancel(true)
      .build()
    nm.notify(GROUP.hashCode(), summary)
  }
}

/**
 * A reply typed into a whisper notification. Sends it through the core
 * (NotifyBridge.replyWhisper: the same send the Whispers screen uses), off the
 * main thread, then shows how it went in the notification.
 */
class WhisperReplyReceiver : BroadcastReceiver() {
  override fun onReceive(context: Context, intent: Intent) {
    val text = RemoteInput.getResultsFromIntent(intent)?.getCharSequence(WhisperNotifier.KEY_REPLY)
      ?.toString()?.trim().orEmpty()
    val item = WhisperNotifier.Item(
      key = intent.getStringExtra("key").orEmpty(),
      id = intent.getStringExtra("id").orEmpty(),
      userId = intent.getStringExtra("user_id").orEmpty(),
      login = intent.getStringExtra("login").orEmpty(),
      name = intent.getStringExtra("name").orEmpty(),
      avatar = intent.getStringExtra("avatar").orEmpty(),
      text = text,
      atMs = System.currentTimeMillis(),
    )
    if (text.isEmpty() || item.userId.isEmpty()) {
      WhisperNotifier.replyFailed(context, item, "Type a message to send.")
      return
    }
    val app = context.applicationContext
    val pending = goAsync()
    Thread {
      try {
        val reason = try {
          NotifyBridge.replyWhisper(
            app.dataDir.absolutePath,
            item.userId,
            item.login,
            item.name,
            item.avatar,
            text,
          )
        } catch (e: Throwable) {
          Log.e(NotifyRenderer.TAG, "whisper reply JNI failed", e)
          "Failed to send message. Please try again."
        }
        Log.i(NotifyRenderer.TAG, "whisper reply -> ${if (reason.isEmpty()) "sent" else "not sent"}")
        if (reason.isEmpty()) {
          WhisperNotifier.replied(app, item, text)
        } else {
          WhisperNotifier.replyFailed(app, item, reason)
        }
      } finally {
        pending.finish()
      }
    }.start()
  }
}
