package app.streamnook

import android.app.NotificationChannel
import android.app.NotificationManager

/**
 * Android notification categories.
 *
 * These ids are duplicated in `src/mobile/notifications.ts`, which creates the
 * same channels through the Tauri plugin when the web shell boots. The
 * duplication is deliberate: the background worker can run in a process where
 * the shell has never started, so it cannot assume the channels exist, and it
 * has no way to reach the TypeScript definition. Keep the two in step.
 *
 * Channels are IMMUTABLE once created. Android ignores later changes to an
 * existing channel's name or importance, so changing either means minting a new
 * id, and the user's own tweaks to sound and importance always win.
 */
object NotifyChannels {
  const val LIVE = "live-channels"
  const val DROPS = "drops"
  const val BADGES = "badges"
  const val POINTS = "channel-points"

  /** Idempotent: creating a channel that already exists is a no-op. */
  fun ensure(nm: NotificationManager) {
    create(nm, LIVE, "Channels going live", NotificationManager.IMPORTANCE_HIGH)
    create(nm, DROPS, "Drops", NotificationManager.IMPORTANCE_DEFAULT)
    create(nm, BADGES, "New badges", NotificationManager.IMPORTANCE_DEFAULT)
    // Quiet on purpose: this can fire every quarter hour while watching, so it
    // belongs in the shade rather than on top of whatever you are doing.
    create(nm, POINTS, "Channel points", NotificationManager.IMPORTANCE_LOW)
  }

  private fun create(nm: NotificationManager, id: String, name: String, importance: Int) {
    if (nm.getNotificationChannel(id) == null) {
      nm.createNotificationChannel(NotificationChannel(id, name, importance))
    }
  }
}
