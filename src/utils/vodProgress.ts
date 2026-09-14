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

/** "1h 54m left" / "12m left" / "Under a minute left" for a Continue Watching
 *  card. Remaining time is what a viewer deciding whether to resume actually
 *  weighs, so it leads the card; the exact resume point stays in the label. */
export function formatRemaining(positionSecs: number, durationSecs: number): string | null {
  if (!Number.isFinite(durationSecs) || durationSecs <= 0) return null;
  const s = Math.max(0, Math.round(durationSecs - Math.max(0, positionSecs)));
  if (s < 60) return 'Under a minute left';
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (h > 0) return m > 0 ? `${h}h ${m}m left` : `${h}h left`;
  return `${m}m left`;
}

/** What a media card is showing, for its corner pill. */
export type MediaKind = 'clip' | 'vod' | 'highlight' | 'upload';

/** Helix `type` on a video to the card's kind. */
export function mediaKindOfVideo(type: string | undefined): MediaKind {
  if (type === 'highlight') return 'highlight';
  if (type === 'upload') return 'upload';
  return 'vod';
}

/** Helix's "7h19m49s" as whole seconds; null for anything not that shape. */
export function parseHelixDuration(s: string | undefined): number | null {
  if (!s) return null;
  const m = /^(?:(\d+)h)?(?:(\d+)m)?(?:(\d+)s)?$/.exec(s.trim());
  if (!m || m[0] === '') return null;
  return Number(m[1] ?? 0) * 3600 + Number(m[2] ?? 0) * 60 + Number(m[3] ?? 0);
}

/** One clock-style length for every VOD card ("7:19:49"), whichever of the
 *  two shapes Twitch handed us. Falls back to the raw string so a card never
 *  loses its length over a format it does not know. */
export function videoDurationLabel(video: { duration: string; length_seconds?: number }): string {
  const secs =
    video.length_seconds && video.length_seconds > 0
      ? video.length_seconds
      : parseHelixDuration(video.duration);
  return secs != null ? formatVodTime(secs) : video.duration;
}

/** "Sep 12, 2026" for a card's date slot; empty for an unparseable stamp. */
export function formatCardDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' });
}

/** The image Twitch serves for a VOD that has no usable thumbnail yet. */
export const VOD_FALLBACK_THUMB =
  'https://vod-secure.twitch.tv/_404/404_processing_320x180.png';

/** A VOD thumbnail URL ready to put in an `src`.
 *
 *  Two Twitch quirks, both of which 404 if ignored: the URL arrives with
 *  literal `%{width}` / `%{height}` placeholders, and a VOD that is still
 *  being processed returns a `404_processing` placeholder image (verified
 *  2026-09-12 on a mid-broadcast archive). Both end up as the fallback. */
export function vodThumbUrl(url: string | undefined, width = 440, height = 248): string {
  if (!url || url.includes('404_processing')) return VOD_FALLBACK_THUMB;
  return url.replace('%{width}', String(width)).replace('%{height}', String(height));
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
