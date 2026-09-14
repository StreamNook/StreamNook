package app.streamnook

import android.util.Log
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import kotlinx.coroutines.runBlocking
import java.io.File

/**
 * The push lane's receiver. Deliberately a dumb renderer: whether this alert
 * should show at all (enabled, category toggle, muted channel, already
 * announced by the poll lane) is decided by Rust in `claimShow`, atomically
 * against the same state file the WorkManager poll uses. That single authority
 * is what makes a push+poll overlap render exactly one notification.
 *
 * Runs on FCM's background thread; `runBlocking` here only covers the avatar
 * fetch, well inside the handler's time budget.
 */
class SNMessagingService : FirebaseMessagingService() {

  override fun onMessageReceived(message: RemoteMessage) {
    val data = message.data
    if (data["type"] != "live") return
    val channelId = data["channel_id"] ?: return
    val startedAt = data["started_at"] ?: ""
    val login = data["login"] ?: ""

    val show = try {
      NotifyBridge.claimShow(dataDir.absolutePath, channelId, startedAt, login)
    } catch (e: Throwable) {
      // UnsatisfiedLinkError is an Error, not an Exception; catching Throwable
      // is deliberate, same as the worker. No claim, no notification.
      Log.e(NotifyRenderer.TAG, "claimShow failed", e)
      false
    }
    Log.i(NotifyRenderer.TAG, "push $login -> ${if (show) "post" else "suppressed"}")
    if (!show) return

    runBlocking {
      NotifyRenderer.postAll(
        this@SNMessagingService,
        listOf(
          NotifyRenderer.Item(
            channelId = channelId,
            login = login,
            title = data["title"] ?: "$login is live",
            body = data["body"] ?: "",
            avatar = data["avatar"]?.takeIf { it.isNotEmpty() },
            channel = NotifyChannels.LIVE,
          )
        ),
      )
    }
  }

  override fun onNewToken(token: String) {
    writeToken(this, token)
  }

  companion object {
    /**
     * The token lives in its own file under the Rust-owned data dir, single
     * writer (Kotlin), read by the `push_register` command. Same pattern the
     * retired notify_ping used: separate files per writer, no shared JSON.
     */
    fun writeToken(context: android.content.Context, token: String) {
      try {
        val dir = File(context.dataDir, "StreamNook")
        if (!dir.exists()) dir.mkdirs()
        File(dir, "fcm_token").writeText(token)
        Log.i(NotifyRenderer.TAG, "fcm token stored")
      } catch (e: Exception) {
        Log.w(NotifyRenderer.TAG, "fcm token write failed", e)
      }
    }
  }
}
