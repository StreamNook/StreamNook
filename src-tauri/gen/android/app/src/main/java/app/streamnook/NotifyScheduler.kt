package app.streamnook

import android.content.Context
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import java.util.concurrent.TimeUnit

/**
 * Schedules and cancels the background notification poll.
 *
 * Fifteen minutes is the floor WorkManager enforces, and how long the job
 * actually waits past that depends on the app's standby bucket. In the Rare and
 * Restricted buckets Android withholds network from jobs entirely, so the
 * battery-optimisation exemption is what decides whether this feature works at
 * all, not this interval.
 */
object NotifyScheduler {
  private const val WORK_NAME = "sn_live_notifications"

  /**
   * Idempotent: safe to call on every launch and whenever the setting changes.
   *
   * UPDATE rather than KEEP because the interval is user-configurable and KEEP
   * would silently ignore a changed one. UPDATE also leaves a currently running
   * worker alone and keeps the original enqueue time, so re-scheduling on every
   * launch does not push the next run further out each time.
   */
  fun schedule(context: Context, intervalMinutes: Long) {
    val interval = intervalMinutes.coerceAtLeast(15)
    val request = PeriodicWorkRequestBuilder<NotifyWorker>(interval, TimeUnit.MINUTES)
      .setConstraints(
        Constraints.Builder()
          .setRequiredNetworkType(NetworkType.CONNECTED)
          .build()
      )
      .build()
    WorkManager.getInstance(context).enqueueUniquePeriodicWork(
      WORK_NAME,
      ExistingPeriodicWorkPolicy.UPDATE,
      request,
    )
  }

  fun cancel(context: Context) {
    WorkManager.getInstance(context).cancelUniqueWork(WORK_NAME)
  }

  /**
   * One immediate run of the same worker, for the moment the app comes back to
   * the foreground: the periodic slot may be most of its interval away, and the
   * user is looking at the phone right now.
   *
   * Same worker, same state file, so it structurally cannot double-post. KEEP
   * coalesces a burst of resumes into one pending run. Plain work rather than
   * expedited on purpose: pre-Android-12 expedited work runs as a foreground
   * service and throws without a getForegroundInfo implementation, while a
   * plain one-shot in the Active bucket starts within seconds anyway.
   */
  fun runOnce(context: Context) {
    val request = OneTimeWorkRequestBuilder<NotifyWorker>()
      .setConstraints(
        Constraints.Builder()
          .setRequiredNetworkType(NetworkType.CONNECTED)
          .build()
      )
      .build()
    WorkManager.getInstance(context).enqueueUniqueWork(
      "${WORK_NAME}_now",
      ExistingWorkPolicy.KEEP,
      request,
    )
  }
}
