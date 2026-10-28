// The words Your songs uses for time and place. Pure, so they are tested
// without a page (songWords.test.ts).
import type { PlayerSource } from './activeVideo';

/** The channel a song was heard on, by its shown name. */
export function channelOf(source: PlayerSource): string {
  return source.channel_name || source.channel_login;
}

/** The mark beside the channel when it was not heard live: "VOD" or "Clip". */
export function mediaMark(source: PlayerSource): 'VOD' | 'Clip' | null {
  if (source.media === 'video') return 'VOD';
  if (source.media === 'clip') return 'Clip';
  return null;
}

/** A video time as players show it: "1:23:45", "4:05". */
function clock(seconds: number): string {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = seconds % 60;
  const two = (n: number) => String(n).padStart(2, '0');
  return h > 0 ? `${h}:${two(m)}:${two(s)}` : `${m}:${two(s)}`;
}

/** How far in it was heard: "2h 14m in" for a broadcast, "at 1:23:45" on a
 *  VOD, null when that was not kept. */
export function heardInto(source: PlayerSource): string | null {
  const secs = source.position_secs;
  if (secs == null || !Number.isFinite(secs) || secs < 0) return null;
  const whole = Math.floor(secs);
  if (source.media === 'video') return `at ${clock(whole)}`;
  if (source.media === 'clip') return null;
  const minutes = Math.floor(whole / 60);
  if (minutes < 1) return 'at the start';
  const hours = Math.floor(minutes / 60);
  if (hours === 0) return `${minutes}m in`;
  return minutes % 60 ? `${hours}h ${minutes % 60}m in` : `${hours}h in`;
}

/** How long ago, short: "just now", "12m ago", "5h ago", "3d ago", then the
 *  date. `compact` gives another year's date as month and year ("Oct 2025"),
 *  for a slot too narrow for the day as well. */
export function heardAgo(ms: number, now: number = Date.now(), compact = false): string {
  const seconds = Math.max(0, Math.round((now - ms) / 1000));
  if (seconds < 60) return 'just now';
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d ago`;
  return dateOf(ms, now, compact);
}

function dateOf(ms: number, now: number, compact = false): string {
  const date = new Date(ms);
  if (date.getFullYear() === new Date(now).getFullYear()) {
    return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
  }
  return date.toLocaleDateString('en-US', compact ? { month: 'short', year: 'numeric' } : { month: 'short', day: 'numeric', year: 'numeric' });
}

/** The day it was heard: "Today", "Yesterday", else the date ("Oct 26"),
 *  with the year when it is not this year (`compact`: as in heardAgo). */
export function heardDay(ms: number, now: number = Date.now(), compact = false): string {
  const day = (t: number) => {
    const d = new Date(t);
    return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  };
  const today = day(now);
  const then = day(ms);
  if (then === today) return 'Today';
  const yesterday = new Date(today);
  yesterday.setDate(yesterday.getDate() - 1);
  if (then === yesterday.getTime()) return 'Yesterday';
  return dateOf(ms, now, compact);
}

/** The album to show under a song, or null when it only repeats the title
 *  (a single: "Solita" on "Solita - Single"). */
export function albumLine(album: string | null, title: string): string | null {
  if (!album) return null;
  const words = (s: string) => s.toLowerCase().replace(/\s+/g, ' ').trim();
  const bare = words(album).replace(/\s*[-–]\s*(single|ep)$/, '');
  return bare === words(title) ? null : album;
}

/** The time of day it was heard: "9:14 PM". NFKC makes the narrow space
 *  newer ICU puts before the PM an ordinary one. */
export function heardClock(ms: number): string {
  return new Date(ms).toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' }).normalize('NFKC');
}
