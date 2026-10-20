// A channel's videos and clips for the phone's Channel sheet, and the VOD
// player's clock math. The rows come from Rust `channel_media`, worded there;
// this file only builds the arguments and joins the lines, exactly as the
// iPhone's SNKit `MediaShelf` / `MediaRow` do. Type imports only, so vitest can load it.

import type { VodStoryboard } from '../../types';

/** One row of `channel_media`, every label worded by the core. */
export interface MediaRow {
  /** "video" or "clip". */
  kind: 'video' | 'clip';
  /** The video id or the clip slug. */
  id: string;
  title: string;
  thumbnailUrl: string;
  /** What `start_stream` (videos) or the clip sheet (clips) opens. */
  url: string;
  lengthLabel: string;
  ageLabel: string;
  viewsLabel: string;
  /** The category (videos), or "Clipped by <name>" (clips). */
  byline: string | null;
  /** How far the viewer got, 0 to 1; null when never opened. */
  progress: number | null;
  watched: boolean;
  /** The broadcast is still live, so its video is still growing. */
  recording: boolean;
  /** A clip's parent video and where in it the clip starts. */
  vodId: string | null;
  vodOffset: number | null;
}

export interface MediaPage {
  rows: MediaRow[];
  cursor: string | null;
}

export type MediaTab = 'videos' | 'clips';
export type VideoKind = 'archive' | 'highlight' | 'upload';
export type VideoSort = 'recent' | 'popular';
export type ClipPeriod = '24h' | '7d' | '30d' | 'all';

export const VIDEO_KINDS: { value: VideoKind; label: string }[] = [
  { value: 'archive', label: 'Past broadcasts' },
  { value: 'highlight', label: 'Highlights' },
  { value: 'upload', label: 'Uploads' },
];

export const VIDEO_SORTS: { value: VideoSort; label: string }[] = [
  { value: 'recent', label: 'Recent' },
  { value: 'popular', label: 'Popular' },
];

export const CLIP_PERIODS: { value: ClipPeriod; label: string }[] = [
  { value: '24h', label: '24 hours' },
  { value: '7d', label: '7 days' },
  { value: '30d', label: '30 days' },
  { value: 'all', label: 'All time' },
];

/** What the sheet lists: videos by kind and sort, or clips by period. */
export type MediaShelf =
  | { tab: 'videos'; kind: VideoKind; sort: VideoSort }
  | { tab: 'clips'; period: ClipPeriod };

export function shelfKey(shelf: MediaShelf): string {
  return shelf.tab === 'videos' ? `videos:${shelf.kind}:${shelf.sort}` : `clips:${shelf.period}`;
}

/** The `channel_media` arguments for a shelf. */
export function shelfArgs(shelf: MediaShelf, channelId: string, cursor: string | null): Record<string, unknown> {
  return shelf.tab === 'videos'
    ? { channelId, kind: 'videos', filter: shelf.kind, sort: shelf.sort, cursor }
    : { channelId, kind: 'clips', filter: shelf.period, cursor };
}

/** What an empty shelf says. */
export function emptyLine(shelf: MediaShelf): string {
  if (shelf.tab === 'clips') return 'No clips in this period';
  const kind = VIDEO_KINDS.find((k) => k.value === shelf.kind)?.label ?? 'Videos';
  return `No ${kind.toLowerCase()} yet`;
}

const joinDot = (parts: (string | null | undefined)[]) => parts.filter((p): p is string => !!p).join(' · ');

/** The row's second line: age and length for a video, who clipped it and when for a clip. */
export function metaLine(row: MediaRow): string {
  return row.kind === 'clip' ? joinDot([row.byline, row.ageLabel]) : joinDot([row.ageLabel, row.lengthLabel]);
}

/** The third line: views and the category for a video, length and views for a clip. */
export function countLine(row: MediaRow): string {
  return row.kind === 'clip' ? joinDot([row.lengthLabel, row.viewsLabel]) : joinDot([row.viewsLabel, row.byline]);
}

/** Adds a page under what is shown, without a row twice. */
export function mergePage(shown: MediaRow[], page: MediaRow[]): MediaRow[] {
  const known = new Set(shown.map((r) => r.id));
  return shown.concat(page.filter((r) => !known.has(r.id)));
}

/** A playhead as the seek bar shows it: m:ss, or h:mm:ss from an hour, or when
 *  `like` (the length beside it) runs an hour or more, so the pair keeps one shape. */
export function clock(seconds: number, like?: number): string {
  const s = Math.max(0, Math.floor(Number.isFinite(seconds) ? seconds : 0));
  const h = Math.floor(s / 3600);
  const wide = h > 0 || (like != null && like >= 3600);
  const sec = String(s % 60).padStart(2, '0');
  return wide ? `${h}:${String(Math.floor((s % 3600) / 60)).padStart(2, '0')}:${sec}` : `${Math.floor(s / 60)}:${sec}`;
}

/** Where a seek lands: inside the video, a second short of its end (as the iPhone). */
export function seekTarget(seconds: number, length: number): number {
  return Math.max(0, length > 0 ? Math.min(seconds, length - 1) : seconds);
}

/** Where a skip of `by` seconds lands. */
export function skipTarget(position: number, by: number, length: number): number {
  return seekTarget(position + by, length);
}

/** The playhead for a press at `x` across a bar from `left` that is `width` wide. */
export function seekAt(x: number, left: number, width: number, length: number): number {
  if (width <= 0 || length <= 0) return 0;
  return Math.min(Math.max((x - left) / width, 0), 1) * length;
}

/** One seek-preview frame: the sheet to load and where the frame sits in it. */
export interface StoryboardFrame {
  url: string;
  x: number;
  y: number;
  width: number;
  height: number;
  sheetWidth: number;
  sheetHeight: number;
}

/** The frame for `seconds`, from the largest variant no wider than `maxWidth`
 *  pixels (SNKit `Storyboard.frame`). */
export function storyboardFrame(sb: VodStoryboard | undefined | null, seconds: number, maxWidth = 320): StoryboardFrame | null {
  if (!sb) return null;
  const fits = sb.variants.filter((v) => v.width <= maxWidth);
  const v = fits.length ? fits[fits.length - 1] : sb.variants[0];
  if (!v || v.interval_secs <= 0 || v.rows <= 0 || v.cols <= 0 || v.count <= 0) return null;
  const index = Math.min(v.count - 1, Math.max(0, Math.floor(seconds / v.interval_secs)));
  const perSheet = v.rows * v.cols;
  const sheet = Math.floor(index / perSheet);
  const image = v.images[sheet];
  if (!image) return null;
  const cell = index % perSheet;
  return {
    url: sb.base_url + image,
    x: (cell % v.cols) * v.width,
    y: Math.floor(cell / v.cols) * v.height,
    width: v.width,
    height: v.height,
    sheetWidth: v.cols * v.width,
    sheetHeight: v.rows * v.height,
  };
}

/** "Recorded Sep 28" while a VOD plays, with the year when it is not this one. */
export function recordedLine(recordedAt: string | undefined, now = new Date()): string {
  const date = recordedAt ? new Date(recordedAt) : null;
  if (!date || Number.isNaN(date.getTime())) return 'Recorded';
  const sameYear = date.getFullYear() === now.getFullYear();
  return `Recorded ${date.toLocaleDateString('en-US', {
    month: 'short',
    day: 'numeric',
    ...(sameYear ? {} : { year: 'numeric' }),
  })}`;
}

/** The speeds a VOD offers, as on the iPhone. */
export const VOD_RATES = [1, 1.25, 1.5, 2] as const;

/** "1x", "1.25x", "1.5x", "2x", as the iPhone's `PlayerText.rate`. */
export function rateLabel(rate: number): string {
  return `${Math.round(rate * 100) / 100}x`;
}
