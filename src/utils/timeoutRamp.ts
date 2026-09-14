// Continuous timeout duration from a drag distance.
//
// Lifted out of ModerationDragLayer so the mobile fan-out dials timeouts on the
// exact same curve as the desktop drag layer. Two implementations of this would
// drift, and "how long did that drag actually time them out for" is not a thing
// to be approximate about.

/** Twitch timeout range: 1s up to the 14-day max; longer is a ban. */
export const MAX_TIMEOUT_SECS = 1209600;

/** Drag-out distance that reaches the top of the range. */
export const TIMEOUT_RANGE_PX = 260;

/** Round to a tidy value whose granularity grows with magnitude. */
export function snapDuration(s: number): number {
  let v: number;
  if (s < 60) v = Math.round(s / 5) * 5;
  else if (s < 3600) v = Math.round(s / 60) * 60;
  else if (s < 86400) v = Math.round(s / 1800) * 1800;
  else v = Math.round(s / 86400) * 86400;
  return Math.min(MAX_TIMEOUT_SECS, Math.max(1, v));
}

/**
 * Drag-out px -> seconds, on a steep 10th-power ramp: most of the travel covers
 * short timeouts and only the last stretch shoots to the 14-day max, which is
 * what makes a single gesture usable for both "5 seconds" and "two weeks".
 */
export function timeoutSecsFromDistance(px: number): number {
  const ratio = Math.min(1, Math.max(0, px) / TIMEOUT_RANGE_PX);
  return snapDuration(Math.pow(ratio, 10) * MAX_TIMEOUT_SECS);
}

/** Human-readable duration (e.g. "45s", "10m", "1h 30m", "2d"). */
export function formatDuration(s: number): string {
  if (s < 60) return `${Math.round(s)}s`;
  if (s < 3600) return `${Math.round(s / 60)}m`;
  if (s < 86400) {
    const h = Math.floor(s / 3600);
    const m = Math.round((s % 3600) / 60);
    return m ? `${h}h ${m}m` : `${h}h`;
  }
  const d = Math.floor(s / 86400);
  const h = Math.round((s % 86400) / 3600);
  return h ? `${d}d ${h}h` : `${d}d`;
}

/**
 * Magnitude tier of a duration. The mobile fan ticks a haptic only when this
 * changes, because ticking every snap while dialling buzzes continuously.
 */
export function durationTier(s: number): 0 | 1 | 2 | 3 {
  if (s < 60) return 0;
  if (s < 3600) return 1;
  if (s < 86400) return 2;
  return 3;
}
