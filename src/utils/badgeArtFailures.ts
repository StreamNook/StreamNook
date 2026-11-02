import { useSyncExternalStore } from 'react';

// Badge art that finally failed to load (after any retry). Rows drop these
// before folding, so a dead image never leaves a gap or a "+N" that opens empty.
// A failure counts for RETRY_MS only, and coming back online clears them all,
// so a short network drop never hides a badge for the rest of the session.
// A row re-renders only when one of ITS urls fails: its snapshot is the
// failed subset of its own urls, joined, which stays equal otherwise.
const failedAt = new Map<string, number>();
const FAILED_MAX = 2000;
export const RETRY_MS = 60_000;
const listeners = new Set<() => void>();
const notify = () => listeners.forEach((l) => l());

const isFailed = (url: string) => {
  const at = failedAt.get(url);
  return at !== undefined && Date.now() - at < RETRY_MS;
};

// One timer at a time, for the earliest expiry: mounted rows (the pinned card,
// the settings preview, rows scrolled back to) learn a badge may load again.
let expiryTimer: ReturnType<typeof setTimeout> | null = null;
function scheduleExpiry(): void {
  if (expiryTimer !== null || failedAt.size === 0) return;
  const earliest = Math.min(...failedAt.values());
  expiryTimer = setTimeout(() => {
    expiryTimer = null;
    const now = Date.now();
    for (const [url, at] of failedAt) if (now - at >= RETRY_MS) failedAt.delete(url);
    notify();
    scheduleExpiry();
  }, Math.max(0, earliest + RETRY_MS - Date.now()) + 50);
}

export function markBadgeArtFailed(url: string | undefined): void {
  if (!url || isFailed(url)) return;
  if (failedAt.size >= FAILED_MAX) failedAt.delete(failedAt.keys().next().value as string);
  failedAt.set(url, Date.now());
  notify();
  scheduleExpiry();
}

export function isBadgeArtFailed(url: string | undefined): boolean {
  return !!url && isFailed(url);
}

if (typeof window !== 'undefined') {
  window.addEventListener('online', () => {
    if (failedAt.size === 0) return;
    failedAt.clear();
    notify();
  });
}

export const subscribeBadgeArtFailures = (l: () => void) => {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
};
const subscribe = subscribeBadgeArtFailures;

/** The failed ones among `urls`, as one stable string ("" when none). */
export function failedAmong(urls: readonly string[]): string {
  let out = '';
  for (const u of urls) if (u && isFailed(u)) out += `${u}\n`;
  return out;
}

/** Subscribes a row to its own badge urls; returns `failedAmong(urls)`. */
export function useFailedBadgeArt(urls: readonly string[]): string {
  return useSyncExternalStore(subscribe, () => failedAmong(urls), () => '');
}
