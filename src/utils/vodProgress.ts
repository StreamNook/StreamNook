import type { VodProgressSummary } from '../types';

/**
 * Presentation helpers for the Rust-owned VOD watch position. Rust joins
 * `progress` onto every video row it returns and applies the resume policy;
 * these only turn that into a bar width and a label.
 */

/** 0..1 of the video watched, or null when there is nothing to draw. */
export function vodProgressFraction(
  progress: VodProgressSummary | undefined,
  lengthSeconds: number | undefined,
): number | null {
  if (!progress) return null;
  if (progress.completed) return 1;
  const denom = progress.duration_secs > 0 ? progress.duration_secs : (lengthSeconds ?? 0);
  if (denom <= 0) return null;
  const frac = progress.position_secs / denom;
  if (!Number.isFinite(frac) || frac < 0.005) return null;
  return Math.min(1, frac);
}

/** "1:23:45" / "4:05" style label for a position in seconds. */
export function formatVodTime(secs: number): string {
  const s = Math.max(0, Math.floor(Number.isFinite(secs) ? secs : 0));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  return h > 0
    ? `${h}:${m.toString().padStart(2, '0')}:${r.toString().padStart(2, '0')}`
    : `${m}:${r.toString().padStart(2, '0')}`;
}

/** "Watched" or "Resume at h:mm:ss" for a card's meta row. Null when the video
 *  was never watched far enough to matter (mirrors Rust's 30 s floor). */
export function vodProgressLabel(progress: VodProgressSummary | undefined): string | null {
  if (!progress) return null;
  if (progress.completed) return 'Watched';
  if (progress.position_secs < 30) return null;
  return `Resume at ${formatVodTime(progress.position_secs)}`;
}

/** "42m ago" / "1h 05m ago" / "just now" for a distance behind live. */
export function formatAgo(behindSecs: number): string {
  const s = Math.max(0, Math.round(behindSecs));
  if (s < 60) return 'just now';
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (h > 0) return `${h}h ${m.toString().padStart(2, '0')}m ago`;
  return `${m}m ago`;
}
