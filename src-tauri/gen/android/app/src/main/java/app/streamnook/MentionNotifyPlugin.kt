package app.streamnook

import android.app.Activity
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.Person
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin

/**
 * Posts a mention notification the Rust core already decided to show
 * (services::mention_inbox checked the setting, streamer mode, the per-chat
 * cooldown, replay age, and that the chat is not on screen). Called from
 * android_notify.rs, never from the page: wry pauses the WebView whenever the
 * app leaves the screen, which is exactly when a mention matters most.
 *
 * Arguments are the Rust `MentionEntry`, read loosely so a field added later
 * never fails the call.
 */
@TauriPlugin
class MentionNotifyPlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun post(invoke: Invoke) {
    val a = invoke.getArgs()
    val item = MentionNotifier.Item(
      id = a.optString("id", ""),
      room = a.optString("room", ""),
      platform = a.optString("platform", "twitch"),
      channel = a.optString("channel_login", ""),
      senderLogin = a.optString("sender_login", ""),
      senderName = a.optString("sender_name", ""),
      text = a.optString("text", ""),
      kind = a.optString("kind", "mention"),
      atMs = a.optLong("at_ms", System.currentTimeMillis()),
    )
    invoke.resolve()
    val context = activity.applicationContext
    // Off the caller's thread: posting is quick, but never worth blocking the
    // chat lane that emitted the event.
    Thread { MentionNotifier.post(context, item) }.start()
  }

  /**
   * A whisper the core decided to announce (services::whisper_inbox: the
   * setting, streamer mode, not the conversation on screen). The argument is
   * the Rust `WhisperUpdate`: `key`, `meta` and the incoming `message`.
   */
  @Command
  fun postWhisper(invoke: Invoke) {
    val a = invoke.getArgs()
    val meta = a.optJSONObject("meta")
    val message = a.optJSONObject("message")
    invoke.resolve()
    if (meta == null || message == null || message.optBoolean("is_sent", false)) return
    val item = WhisperNotifier.Item(
      key = a.optString("key", ""),
      id = message.optString("id", ""),
      userId = meta.optString("user_id", ""),
      login = meta.optString("user_login", message.optString("from_user_login", "")),
      name = meta.optString("user_name", "").ifEmpty { message.optString("from_user_name", "") },
      avatar = if (meta.isNull("profile_image_url")) "" else meta.optString("profile_image_url", ""),
      text = message.optString("message", ""),
      atMs = message.optLong("timestamp", System.currentTimeMillis()),
    )
    val context = activity.applicationContext
    Thread { WhisperNotifier.post(context, item) }.start()
  }
}

/**
 * The mention notification itself: one conversation per chat in messaging
 * style, newest message last, the tap opening that mention (the chat, then the
 * jump to the message) through MainActivity.
 */
object MentionNotifier {
  const val EXTRA_MENTION = "sn_mention"
  private const val GROUP = "app.streamnook.GROUP.mentions"
  /** Messages kept in one chat's notification. */
  private const val KEEP = 5
  /** Message ids already posted, so a message seen twice is posted once. */
  private val recent = ArrayDeque<String>()
  private const val RECENT_MAX = 200

  data class Item(
    val id: String,
    val room: String,
    val platform: String,
    val channel: String,
    val senderLogin: String,
    val senderName: String,
    val text: String,
    val kind: String,
    val atMs: Long,
  )

  @Synchronized
  private fun claim(id: String): Boolean {
    if (recent.contains(id)) return false
    recent.addLast(id)
    while (recent.size > RECENT_MAX) recent.removeFirst()
    return true
  }

  private fun platformLabel(platform: String): String? = when (platform) {
    "twitch" -> null
    "kick" -> "Kick"
    "youtube" -> "YouTube"
    "tiktok" -> "TikTok"
    else -> platform.replaceFirstChar { it.uppercase() }
  }

  fun post(context: Context, m: Item) {
    if (m.id.isEmpty() || m.room.isEmpty() || !claim(m.id)) return
    try {
      val nm = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
      NotifyChannels.ensure(nm)
      // One notification per chat: hashed from the room so a later mention
      // there joins the same conversation instead of stacking a new row.
      val notifId = "mention:${m.room}".hashCode()

      val me = Person.Builder().setName("You").build()
      val sender = Person.Builder()
        .setName(m.senderName.ifEmpty { m.senderLogin })
        .setKey("${m.platform}:${m.senderLogin}")
        .build()
      val existing = nm.activeNotifications.firstOrNull { it.id == notifId }?.notification
      val style = existing?.let { NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(it) }
        ?: NotificationCompat.MessagingStyle(me)
      val title = platformLabel(m.platform)?.let { "${m.channel} on $it" } ?: m.channel
      style.setConversationTitle(title)
      style.setGroupConversation(true)
      style.addMessage(NotificationCompat.MessagingStyle.Message(m.text, m.atMs, sender))
      val messages = style.messages
      while (messages.size > KEEP) messages.removeAt(0)

      val intent = Intent(context, MainActivity::class.java).apply {
        flags = Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP
        putExtra(EXTRA_MENTION, m.id)
      }
      // Per chat, and UPDATE_CURRENT: a tap opens the newest mention there.
      val pending = PendingIntent.getActivity(
        context,
        notifId,
        intent,
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
      )

      val notification = NotificationCompat.Builder(context, NotifyChannels.MENTIONS)
        .setSmallIcon(R.drawable.ic_stat_notify)
        .setStyle(style)
        .setCategory(NotificationCompat.CATEGORY_MESSAGE)
        .setWhen(m.atMs)
        .setShowWhen(true)
        .setAutoCancel(true)
        .setOnlyAlertOnce(false)
        .setGroup(GROUP)
        .setContentIntent(pending)
        .build()
      nm.notify(notifId, notification)

      // Several chats at once collapse into one row in the shade.
      val summary = NotificationCompat.Builder(context, NotifyChannels.MENTIONS)
        .setSmallIcon(R.drawable.ic_stat_notify)
        .setGroup(GROUP)
        .setGroupSummary(true)
        .setAutoCancel(true)
        .build()
      nm.notify(GROUP.hashCode(), summary)
    } catch (e: Exception) {
      // A missing permission or a dead channel costs this one notification.
      Log.w(NotifyRenderer.TAG, "mention notification failed", e)
    }
  }
}
