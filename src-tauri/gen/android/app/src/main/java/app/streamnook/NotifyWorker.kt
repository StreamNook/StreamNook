package app.streamnook

import android.content.Context
import android.util.Log
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import org.json.JSONArray

/**
 * The poll lane: delivers notifications on a schedule, app open or closed.
 *
 * Calls straight into the Rust core over JNI. Rust owns the whole decision
 * (token refresh, which broadcasts are new, what is muted) and hands back a
 * ready-to-post list; rendering is shared with the push receiver via
 * [NotifyRenderer], so both lanes look identical and share notification ids.
 */
class NotifyWorker(
  private val context: Context,
  parameters: WorkerParameters,
) : CoroutineWorker(context, parameters) {

  override suspend fun doWork(): Result {
    val payload = try {
      // getDataDir(), matching what Tauri's app_data_dir() resolves to. Rust
      // verifies it and falls back to files/ rather than trusting this blindly.
      NotifyBridge.pollOnce(context.dataDir.absolutePath)
    } catch (e: Throwable) {
      // UnsatisfiedLinkError is the interesting failure here and it is an Error,
      // not an Exception, so catching Throwable is deliberate.
      Log.e(TAG, "JNI poll failed", e)
      null
    }

    val items = parse(payload)
    // Logged on every path, including the empty one. Rust returns [] for a
    // whole family of perfectly correct outcomes (nobody new is live,
    // notifications are off, not signed in), and without this line a run that
    // deliberately did nothing looks exactly like a run that never happened.
    Log.i(TAG, "poll -> ${if (payload == null) "JNI FAILED" else "${items.size} to post"}")
    if (items.isNotEmpty()) {
      NotifyRenderer.postAll(context, items)
    }

    // Always success. At a 15 minute period a retry buys nothing and spends
    // standby-bucket quota the next run needs.
    return Result.success()
  }

  private fun parse(json: String?): List<NotifyRenderer.Item> {
    if (json.isNullOrBlank()) return emptyList()
    return try {
      val arr = JSONArray(json)
      (0 until arr.length()).mapNotNull { i ->
        val o = arr.optJSONObject(i) ?: return@mapNotNull null
        val id = o.optString("channel_id")
        if (id.isEmpty()) return@mapNotNull null
        NotifyRenderer.Item(
          channelId = id,
          login = o.optString("login"),
          title = o.optString("title"),
          body = o.optString("body"),
          avatar = o.optString("avatar").takeIf { it.isNotEmpty() },
          channel = o.optString("channel").takeIf { it.isNotEmpty() } ?: NotifyChannels.LIVE,
        )
      }
    } catch (e: Exception) {
      Log.e(TAG, "malformed poll payload", e)
      emptyList()
    }
  }

  companion object {
    const val TAG = NotifyRenderer.TAG
    const val EXTRA_CHANNEL = NotifyRenderer.EXTRA_CHANNEL
  }
}
