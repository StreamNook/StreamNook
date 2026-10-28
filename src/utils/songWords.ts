// The words Your songs uses for time and place. Pure, so they are tested
// without a page (songWords.test.ts).
import type { PlayerSource } from './activeVideo';

/** Where a song was heard, as its line says it: the channel's name, and
 *  "VOD" or "clip" when it was not live. */
export function placeOf(source: PlayerSource): string {
  const name = source.channel_name || source.channel_login;
  if (source.media === 'video') return `${name} · VOD`;
  if (source.media === 'clip') return `${name} · clip`;
  return name;
}

/** How long ago, short: "just now", "12m ago", "5h ago", "3d ago", then the date. */
export function heardAgo(ms: number, now: number = Date.now()): string {
  const seconds = Math.max(0, Math.round((now - ms) / 1000));
  if (seconds < 60) return 'just now';
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d ago`;
  const date = new Date(ms);
  const otherYear = date.getFullYear() !== new Date(now).getFullYear();
  return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric', ...(otherYear ? { year: 'numeric' } : {}) });
}

/** A hearing's date and time ("Oct 26, 9:14 PM"), with the year when it is
 *  not this year. */
export function heardAt(ms: number, now: number = Date.now()): string {
  const date = new Date(ms);
  const otherYear = date.getFullYear() !== new Date(now).getFullYear();
  return date.toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    ...(otherYear ? { year: 'numeric' } : {}),
    hour: 'numeric',
    minute: '2-digit',
  });
}
