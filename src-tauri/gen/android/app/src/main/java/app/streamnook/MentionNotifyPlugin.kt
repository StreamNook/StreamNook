package app.streamnook

import android.app.Activity
import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Typeface
import android.text.SpannableStringBuilder
import android.text.Spanned
import android.text.style.StyleSpan
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.Person
import androidx.core.graphics.drawable.IconCompat
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import org.json.JSONObject

/**
 * Posts a mention notification the Rust core already decided to show
 * (services::mention_inbox checked the setting, streamer mode, the per-chat
 * cooldown, replay age, and that the chat is not on screen). Called from
 * android_notify.rs, never from the page: wry pauses the WebView whenever the
 * app leaves the screen, which is exactly when a mention matters most.
 *
 * Arguments are the Rust `MentionNotice` (the `MentionEntry` with its notice
 * words and picture URLs beside it), read loosely so a field added later never
 * fails the call.
 */
@TauriPlugin
class MentionNotifyPlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun post(invoke: Invoke) {
    val a = invoke.getArgs()
    // The words every app's notice uses (mention_inbox::notice_text), beside
    // the entry. Absent from an older core: the entry's own text stands in.
    val notice = a.optJSONObject("notice")
    val item = MentionNotifier.Item(
      id = a.optString("id", ""),
      room = a.optString("room", ""),
      platform = a.optString("platform", "twitch"),
      channel = a.optString("channel_login", ""),
      senderLogin = a.optString("sender_login", ""),
      senderName = a.optString("sender_name", ""),
      text = notice.str("body").ifEmpty { a.optString("text", "") },
      kind = a.optString("kind", "mention"),
      atMs = a.optLong("at_ms", System.currentTimeMillis()),
      quote = notice.str("quote"),
      chatTitle = notice.str("chat_title"),
      senderAvatar = a.str("sender_avatar"),
      channelAvatar = a.str("channel_avatar"),
      pushed = false,
    )
    invoke.resolve()
    val context = activity.applicationContext
    // Off the caller's thread: the pictures are fetched first, and posting is
    // never worth blocking the chat lane that emitted the event.
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
      ownerId = a.optString("owner_id", ""),
      ownerName = message.optString("to_user_name", ""),
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

/** A string field, "" when absent or JSON null (optString turns null into "null"). */
private fun JSONObject?.str(key: String): String =
  if (this == null || isNull(key)) "" else optString(key, "")

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
    /** The message they replied to, already quoted; "" when not a reply. */
    val quote: String = "",
    /** The chat's name as every app shows it ("SomeSlug on Kick"); "" from an older sender. */
    val chatTitle: String = "",
    val senderAvatar: String = "",
    val channelAvatar: String = "",
    /** The picture URLs came in a server push, so they keep to Twitch's picture host. */
    val pushed: Boolean = false,
  )

  @Synchronized
  private fun claim(id: String): Boolean {
    if (recent.contains(id)) return false
    recent.addLast(id)
    while (recent.size > RECENT_MAX) recent.removeFirst()
    return true
  }

  /**
   * The sender's face and the channel's picture, fetched side by side so both
   * fit one 5 s budget (each fetch is bounded by [NotifyFaces]). Either may be
   * null; the notification posts regardless.
   */
  private fun pictures(m: Item): Pair<Bitmap?, Bitmap?> = try {
    runBlocking(Dispatchers.IO) {
      val face = async { NotifyFaces.load(m.senderAvatar, m.pushed) }
      val art = async { NotifyFaces.load(m.channelAvatar, m.pushed) }
      face.await() to art.await()
    }
  } catch (e: Exception) {
    null to null
  }

  /** Their message, under the quoted line it replied to (in italics) when there is one. */
  private fun messageText(m: Item): CharSequence {
    if (m.quote.isEmpty()) return m.text
    return SpannableStringBuilder()
      .append(m.quote, StyleSpan(Typeface.ITALIC), Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
      .append("\n")
      .append(m.text)
  }

  fun post(context: Context, m: Item) {
    if (m.id.isEmpty() || m.room.isEmpty() || !claim(m.id)) return
    // Fetched before the notification is read back, so a slow picture never
    // holds the lock below.
    val (face, art) = pictures(m)
    try {
      val nm = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
      NotifyChannels.ensure(nm)
      // The app and a push can post for the same chat at once; each reads the
      // chat's notification back and adds to it, so one at a time.
      synchronized(this) { postLocked(context, nm, m, face, art) }
    } catch (e: Exception) {
      // A missing permission or a dead channel costs this one notification.
      Log.w(NotifyRenderer.TAG, "mention notification failed", e)
    }
  }

  private fun postLocked(context: Context, nm: NotificationManager, m: Item, face: Bitmap?, art: Bitmap?) {
    // One notification per chat: hashed from the room so a later mention
    // there joins the same conversation instead of stacking a new row.
    val notifId = "mention:${m.room}".hashCode()

    val me = Person.Builder().setName("You").build()
    val senderBuilder = Person.Builder()
      .setName(m.senderName.ifEmpty { m.senderLogin })
      .setKey("${m.platform}:${m.senderLogin}")
    face?.let { senderBuilder.setIcon(IconCompat.createWithBitmap(NotifyFaces.circle(it))) }
    val sender = senderBuilder.build()
    // Older messages come back out of the active notification as they were
    // posted, faces and italics included.
    val existing = nm.activeNotifications.firstOrNull { it.id == notifId }?.notification
    val style = existing?.let { NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(it) }
      ?: NotificationCompat.MessagingStyle(me)
    val title = m.chatTitle.ifEmpty { m.channel.ifEmpty { m.room } }
    style.setConversationTitle(title)
    // Kept on: besides naming the chat above the messages, a group
    // conversation is what lets the channel's picture show on the right.
    style.setGroupConversation(true)
    style.addMessage(NotificationCompat.MessagingStyle.Message(messageText(m), m.atMs, sender))
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

    val builder = NotificationCompat.Builder(context, NotifyChannels.MENTIONS)
      .setSmallIcon(R.drawable.ic_stat_notify)
      .setStyle(style)
      .setCategory(NotificationCompat.CATEGORY_MESSAGE)
      .setWhen(m.atMs)
      .setShowWhen(true)
      .setAutoCancel(true)
      .setOnlyAlertOnce(false)
      .setGroup(GROUP)
      .setContentIntent(pending)
    art?.let { builder.setLargeIcon(it) }
    nm.notify(notifId, builder.build())

    summary(context, nm, notifId, title, m.text)
  }

  /**
   * Several chats at once collapse into one row in the shade. Its expanded
   * form lists the chats newest first, one line each (the chat's name, then
   * its latest message), read back from the mention notifications still up.
   */
  private fun summary(context: Context, nm: NotificationManager, newestId: Int, newestTitle: String, newestText: String) {
    val others = nm.activeNotifications
      .filter {
        it.id != newestId &&
          it.notification.group == GROUP &&
          (it.notification.flags and Notification.FLAG_GROUP_SUMMARY) == 0
      }
      .sortedByDescending { it.postTime }
    val lines = ArrayList<CharSequence>()
    lines.add(inboxLine(newestTitle, newestText))
    for (sbn in others) {
      if (lines.size >= KEEP) break
      val style = NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(sbn.notification) ?: continue
      val title = style.conversationTitle?.toString().orEmpty()
      // Just their message: a reply's quoted line sits above it.
      val text = style.messages.lastOrNull()?.text?.toString()?.substringAfterLast('\n').orEmpty()
      if (title.isEmpty() && text.isEmpty()) continue
      lines.add(inboxLine(title, text))
    }
    val chats = 1 + others.size
    val inbox = NotificationCompat.InboxStyle()
    lines.forEach { inbox.addLine(it) }
    if (chats > 1) inbox.setSummaryText("From $chats chats")

    val summary = NotificationCompat.Builder(context, NotifyChannels.MENTIONS)
      .setSmallIcon(R.drawable.ic_stat_notify)
      .setContentTitle(newestTitle)
      .setContentText(newestText)
      .setSubText("Mentions")
      .setStyle(inbox)
      .setGroup(GROUP)
      .setGroupSummary(true)
      // The chat's own notification already alerted; its summary never does.
      .setGroupAlertBehavior(NotificationCompat.GROUP_ALERT_CHILDREN)
      .setAutoCancel(true)
      .build()
    nm.notify(GROUP.hashCode(), summary)
  }

  /** "bigstreamer  their message", the chat's name in bold. */
  private fun inboxLine(title: String, text: String): CharSequence {
    val line = SpannableStringBuilder()
    if (title.isNotEmpty()) {
      line.append(title, StyleSpan(Typeface.BOLD), Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
      if (text.isNotEmpty()) line.append("  ")
    }
    return line.append(text)
  }
}
