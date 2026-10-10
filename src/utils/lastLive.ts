/**
 * The one wording for "when was this channel live" and "when is it live
 * next", shared by the offline room, Home's offline cards and the chat
 * pickers, so offline reads the same everywhere. Rust supplies the timestamps;
 * this only words them.
 */

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

function parse(iso: string | null | undefined): number | null {
  if (!iso) return null;
  const t = Date.parse(iso);
  return Number.isNaN(t) ? null : t;
}

/** "just now", "12m ago", "3h ago", "yesterday", "4d ago", "2mo ago", "1y ago". */
export function agoLabel(iso: string | null | undefined, now: number = Date.now()): string | null {
  const t = parse(iso);
  if (t === null) return null;
  const diff = Math.max(0, now - t);
  if (diff < MINUTE) return 'just now';
  if (diff < HOUR) return `${Math.floor(diff / MINUTE)}m ago`;
  if (diff < DAY) return `${Math.floor(diff / HOUR)}h ago`;
  if (diff < 2 * DAY) return 'yesterday';
  if (diff < 30 * DAY) return `${Math.floor(diff / DAY)}d ago`;
  if (diff < 365 * DAY) return `${Math.floor(diff / (30 * DAY))}mo ago`;
  return `${Math.floor(diff / (365 * DAY))}y ago`;
}

/** "Last live 3h ago", or "Offline" when the time is unknown. */
export function lastLiveLabel(iso: string | null | undefined, now: number = Date.now()): string {
  const ago = agoLabel(iso, now);
  return ago ? `Last live ${ago}` : 'Offline';
}

function clock(t: number): string {
  return new Date(t).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });
}

function dayStart(t: number): number {
  const d = new Date(t);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/** A scheduled start: "Now", "Today at 8:00 PM", "Tomorrow at 8:00 PM", "Fri at 8:00 PM". */
export function nextStreamLabel(iso: string | null | undefined, now: number = Date.now()): string | null {
  const t = parse(iso);
  if (t === null) return null;
  if (t <= now) return 'Now';
  const days = Math.round((dayStart(t) - dayStart(now)) / DAY);
  if (days === 0) return `Today at ${clock(t)}`;
  if (days === 1) return `Tomorrow at ${clock(t)}`;
  const weekday = new Date(t).toLocaleDateString(undefined, { weekday: 'short' });
  return `${weekday} at ${clock(t)}`;
}

/** A broadcast's length: "12h 36m", "45m", "under a minute". */
export function durationLabel(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (h > 0) return m > 0 ? `${h}h ${m}m` : `${h}h`;
  if (m > 0) return `${m}m`;
  return 'under a minute';
}
