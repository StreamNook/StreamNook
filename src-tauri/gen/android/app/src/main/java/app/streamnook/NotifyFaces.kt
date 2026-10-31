package app.streamnook

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.BitmapShader
import android.graphics.Canvas
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.Shader
import android.os.SystemClock
import android.util.Log
import java.io.ByteArrayOutputStream
import java.net.HttpURLConnection
import java.net.URL

/**
 * The faces on a chat notification (mentions, whispers): the sender's picture
 * and the channel's. One loader so every chat notification fetches pictures
 * under the same bounds.
 *
 * Best effort throughout: any failure returns null and the notification posts
 * without that picture. Nothing here throws. Call it off the main thread.
 */
internal object NotifyFaces {
  /** Edge of the decoded picture, in pixels. Notifications draw faces small. */
  private const val SIZE = 96

  /**
   * The picture at [url], square, at most [SIZE] px. [pushed] says where the
   * URL came from: a server push keeps to Twitch's picture host
   * ([AvatarRules.allowed]) with no redirect followed; one the app's own core
   * handed over may be HTTPS on any host. Both are bounded alike: 5 s for the
   * whole fetch, 1 MB, and bytes that start like a PNG, JPEG, GIF or WebP.
   */
  fun load(url: String, pushed: Boolean): Bitmap? {
    if (url.isEmpty()) return null
    val target = if (pushed) AvatarRules.allowed(url) else AvatarRules.https(url)
    if (target == null) {
      Log.w(NotifyRenderer.TAG, "face skipped: not a fetchable picture URL")
      return null
    }
    return try {
      val bytes = fetch(target, followRedirects = !pushed) ?: return null
      if (!looksLikeImage(bytes)) return null
      decode(bytes)
    } catch (e: Throwable) {
      // Throwable on purpose: a decode that runs out of memory costs the face,
      // never the notification.
      Log.w(NotifyRenderer.TAG, "face fetch failed", e)
      null
    }
  }

  /** [src] cut to a circle, for a person's icon. */
  fun circle(src: Bitmap): Bitmap {
    val size = minOf(src.width, src.height)
    val out = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
    val shader = BitmapShader(src, Shader.TileMode.CLAMP, Shader.TileMode.CLAMP)
    shader.setLocalMatrix(
      Matrix().apply { setTranslate(-(src.width - size) / 2f, -(src.height - size) / 2f) },
    )
    val paint = Paint(Paint.ANTI_ALIAS_FLAG or Paint.FILTER_BITMAP_FLAG).apply { this.shader = shader }
    val radius = size / 2f
    Canvas(out).drawCircle(radius, radius, radius, paint)
    return out
  }

  private fun fetch(target: URL, followRedirects: Boolean): ByteArray? {
    val deadline = SystemClock.elapsedRealtime() + AvatarRules.TIMEOUT_MS
    var conn: HttpURLConnection? = null
    try {
      conn = (target.openConnection() as HttpURLConnection).apply {
        connectTimeout = AvatarRules.TIMEOUT_MS
        readTimeout = AvatarRules.TIMEOUT_MS
        instanceFollowRedirects = followRedirects
        useCaches = false
      }
      if (conn.responseCode != HttpURLConnection.HTTP_OK) return null
      if (conn.contentLengthLong > AvatarRules.MAX_BYTES) return null
      val bytes = ByteArrayOutputStream()
      conn.inputStream.use { input ->
        val buf = ByteArray(16 * 1024)
        while (true) {
          if (SystemClock.elapsedRealtime() > deadline) return null
          val n = input.read(buf)
          if (n < 0) break
          bytes.write(buf, 0, n)
          if (bytes.size() > AvatarRules.MAX_BYTES) return null
        }
      }
      if (SystemClock.elapsedRealtime() > deadline) return null
      return bytes.toByteArray()
    } finally {
      conn?.disconnect()
    }
  }

  /** PNG, JPEG, GIF or WebP by their first bytes, whatever the server called it. */
  private fun looksLikeImage(b: ByteArray): Boolean {
    fun at(i: Int, vararg v: Int) = b.size >= i + v.size && v.indices.all { (b[i + it].toInt() and 0xFF) == v[it] }
    return at(0, 0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A) || // PNG
      at(0, 0xFF, 0xD8, 0xFF) || // JPEG
      at(0, 0x47, 0x49, 0x46, 0x38) || // GIF8
      (at(0, 0x52, 0x49, 0x46, 0x46) && at(8, 0x57, 0x45, 0x42, 0x50)) // RIFF....WEBP
  }

  /**
   * Decoded at roughly [SIZE] (a power-of-two sample first, so a large source
   * never lands in memory whole), cropped to its centered square, then scaled
   * down to [SIZE] when still larger.
   */
  private fun decode(bytes: ByteArray): Bitmap? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
    if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
    var sample = 1
    while (minOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= SIZE) sample *= 2
    val opts = BitmapFactory.Options().apply { inSampleSize = sample }
    val decoded = BitmapFactory.decodeByteArray(bytes, 0, bytes.size, opts) ?: return null
    val edge = minOf(decoded.width, decoded.height)
    val square = if (decoded.width == decoded.height) {
      decoded
    } else {
      Bitmap.createBitmap(decoded, (decoded.width - edge) / 2, (decoded.height - edge) / 2, edge, edge)
    }
    return if (edge > SIZE) Bitmap.createScaledBitmap(square, SIZE, SIZE, true) else square
  }
}
