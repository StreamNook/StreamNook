import { useSyncExternalStore } from 'react';

// Badge art that finally failed to load (after any retry). Rows drop these
// before folding, so a dead image never leaves a gap or a "+N" that opens empty.
// A failure is retried OFF-SCREEN (a detached Image, every RETRY_MS, and at
// once when the network comes back): only art that really loads is cleared, so
// a dead image never pops back into live rows and makes them jump.
// A row re-renders only when one of ITS urls changes state: its snapshot is the
// failed subset of its own urls, joined, which stays equal otherwise.
interface Failure {
  at: number;
  /** Twitch's own copy of a disk-cached badge: a probe tries it too. */
  remote?: string;
}
const failed = new Map<string, Failure>();
const FAILED_MAX = 2000;
export const RETRY_MS = 60_000;
const listeners = new Set<() => void>();

// Many failures at once (going offline) tell rows once.
let notifyQueued = false;
const notify = () => {
  if (notifyQueued) return;
  notifyQueued = true;
  queueMicrotask(() => {
    notifyQueued = false;
    listeners.forEach((l) => l());
  });
};

const loads = (src: string): Promise<boolean> =>
  new Promise((resolve) => {
    if (typeof Image === 'undefined') return resolve(false);
    const img = new Image();
    img.onload = () => resolve(true);
    img.onerror = () => resolve(false);
    img.src = src;
  });

async function probe(url: string): Promise<void> {
  const entry = failed.get(url);
  if (!entry) return;
  const ok = (await loads(url)) || (!!entry.remote && entry.remote !== url && (await loads(entry.remote)));
  if (ok) {
    failed.delete(url);
    notify();
  } else if (failed.has(url)) {
    failed.set(url, { ...entry, at: Date.now() });
  }
}

// One timer at a time, for the earliest retry.
let retryTimer: ReturnType<typeof setTimeout> | null = null;
function scheduleRetry(): void {
  if (retryTimer !== null || failed.size === 0) return;
  let earliest = Infinity;
  for (const f of failed.values()) earliest = Math.min(earliest, f.at);
  retryTimer = setTimeout(() => {
    retryTimer = null;
    const now = Date.now();
    const due = [...failed].filter(([, f]) => now - f.at >= RETRY_MS).map(([url]) => url);
    void Promise.all(due.map(probe)).then(scheduleRetry);
  }, Math.max(0, earliest + RETRY_MS - Date.now()) + 50);
}

export function markBadgeArtFailed(url: string | undefined, remote?: string): void {
  if (!url || failed.has(url)) return;
  if (failed.size >= FAILED_MAX) failed.delete(failed.keys().next().value as string);
  failed.set(url, { at: Date.now(), remote });
  notify();
  scheduleRetry();
}

export function isBadgeArtFailed(url: string | undefined): boolean {
  return !!url && failed.has(url);
}

if (typeof window !== 'undefined') {
  window.addEventListener('online', () => {
    void Promise.all([...failed.keys()].map(probe)).then(scheduleRetry);
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
  for (const u of urls) if (u && failed.has(u)) out += `${u}\n`;
  return out;
}

/** Subscribes a row to its own badge urls; returns `failedAmong(urls)`. */
export function useFailedBadgeArt(urls: readonly string[]): string {
  return useSyncExternalStore(subscribe, () => failedAmong(urls), () => '');
}
