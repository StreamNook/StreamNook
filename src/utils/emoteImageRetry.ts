// Finds a working image for a 7TV emote whose CDN file is not there yet (a
// just-uploaded emote still processing, or a tier or format that was never
// made). One shared retry runs per emote id no matter how many chat rows show
// that emote: it sweeps every tier and format once, then sweeps again on a
// widening schedule while at least one row is mounted, and tells every
// subscribed row the first URL that loads.

export type EmoteImageProbe = (url: string) => Promise<boolean>;

/** Seconds to wait before each later sweep; the retry stops after the last. */
export const RETRY_DELAYS_SECONDS = [5, 15, 45, 120, 300] as const;

const TIERS = ['1x', '2x', '3x', '4x'] as const;

/** Wait before a gave-up emote may start over for a row mounted later. */
const RESTART_AFTER_MS = 300_000;

/** Settled emotes kept so a row mounted later paints at once. */
const MAX_SETTLED = 500;

/**
 * Every CDN file worth trying for an emote, the preferred tier first. Each
 * tier is avif, then webp, then the emote's legacy format when its animation
 * is known (gif when animated, png when static).
 */
export function candidateUrls(id: string, preferredTier = '1x', animated?: boolean): string[] {
  const tiers = [preferredTier, ...TIERS.filter((t) => t !== preferredTier)];
  const formats = ['avif', 'webp', ...(animated === undefined ? [] : [animated ? 'gif' : 'png'])];
  return tiers.flatMap((tier) =>
    formats.map((format) => `https://cdn.7tv.app/emote/${id}/${tier}.${format}`),
  );
}

/** Loads the URL in an image element; true when the browser can decode it. */
export const imageProbe: EmoteImageProbe = (url) =>
  new Promise((resolve) => {
    if (typeof Image === 'undefined') {
      resolve(false);
      return;
    }
    const img = new Image();
    img.referrerPolicy = 'no-referrer';
    // A settled probe lets go of its file: the element is dropped right after,
    // but it would otherwise keep the decoded image until a major collection.
    const settle = (ok: boolean) => {
      resolve(ok);
      img.onload = null;
      img.onerror = null;
      img.removeAttribute('src');
    };
    img.onload = () => settle(true);
    img.onerror = () => settle(false);
    img.src = url;
  });

export interface EmoteImageSubscription {
  /** Called once with the working URL when any file for the emote loads. */
  onResolved: (url: string) => void;
  preferredTier?: string;
  animated?: boolean;
}

interface Entry {
  listeners: Map<symbol, EmoteImageSubscription>;
  resolvedUrl: string | null;
  /** A sweep is awaiting a probe; at most one probe is in flight per emote. */
  running: boolean;
  /** When the last scheduled sweep failed, or null while the retry is alive. */
  gaveUpAt: number | null;
  timer: ReturnType<typeof setTimeout> | null;
}

export interface EmoteImageRetry {
  /** Starts or joins the emote's retry. Returns the function that leaves it. */
  subscribe: (emoteId: string, sub: EmoteImageSubscription) => () => void;
  /** Rows currently waiting on an emote id (0 when idle). */
  subscriberCount: (emoteId: string) => number;
}

export function createEmoteImageRetry(probe: EmoteImageProbe = imageProbe): EmoteImageRetry {
  const entries = new Map<string, Entry>();

  // A settled entry is a resolved emote or one that gave up with no row left.
  const isSettled = (e: Entry) =>
    e.resolvedUrl !== null || (e.gaveUpAt !== null && e.listeners.size === 0);

  const trimSettled = () => {
    let settled = 0;
    for (const e of entries.values()) if (isSettled(e)) settled++;
    for (const [key, e] of entries) {
      if (settled <= MAX_SETTLED) return;
      if (isSettled(e)) {
        entries.delete(key);
        settled--;
      }
    }
  };

  const resolve = (id: string, entry: Entry, url: string) => {
    if (entry.timer !== null) {
      clearTimeout(entry.timer);
      entry.timer = null;
    }
    entry.resolvedUrl = url;
    const listeners = [...entry.listeners.values()];
    entry.listeners.clear();
    // Re-insert so the oldest resolved emote is the one evicted.
    entries.delete(id);
    entries.set(id, entry);
    trimSettled();
    for (const l of listeners) l.onResolved(url);
  };

  /** Tries each candidate in turn, one probe in flight at a time. */
  const sweep = async (id: string, entry: Entry): Promise<string | null> => {
    const first = [...entry.listeners.values()][0];
    for (const url of candidateUrls(id, first?.preferredTier, first?.animated)) {
      if (entry.listeners.size === 0) return null;
      if (await probe(url)) return url;
    }
    return null;
  };

  const run = async (id: string, entry: Entry, round: number): Promise<void> => {
    entry.running = true;
    entry.gaveUpAt = null;
    const found = await sweep(id, entry);
    entry.running = false;
    if (found) {
      resolve(id, entry, found);
      return;
    }
    if (entry.listeners.size === 0) {
      if (entries.get(id) === entry) entries.delete(id);
      return;
    }
    const delay = RETRY_DELAYS_SECONDS[round];
    if (delay === undefined) {
      entry.gaveUpAt = Date.now();
      trimSettled();
      return;
    }
    entry.timer = setTimeout(() => {
      entry.timer = null;
      if (entry.listeners.size === 0) {
        if (entries.get(id) === entry) entries.delete(id);
        return;
      }
      void run(id, entry, round + 1);
    }, delay * 1000);
  };

  const subscribe = (emoteId: string, sub: EmoteImageSubscription): (() => void) => {
    let entry = entries.get(emoteId);
    if (!entry) {
      entry = { listeners: new Map(), resolvedUrl: null, running: false, gaveUpAt: null, timer: null };
      entries.set(emoteId, entry);
    }
    if (entry.resolvedUrl) {
      sub.onResolved(entry.resolvedUrl);
      return () => {};
    }
    const active = entry;
    const token = Symbol(emoteId);
    active.listeners.set(token, sub);
    const idle = !active.running && active.timer === null;
    const mayStart = active.gaveUpAt === null || Date.now() - active.gaveUpAt >= RESTART_AFTER_MS;
    if (idle && mayStart) void run(emoteId, active, 0);

    return () => {
      active.listeners.delete(token);
      if (active.listeners.size > 0 || active.resolvedUrl) return;
      if (active.timer !== null) {
        clearTimeout(active.timer);
        active.timer = null;
      }
      // A sweep still awaiting its probe ends itself and removes the entry.
      if (!active.running && active.gaveUpAt === null && entries.get(emoteId) === active) {
        entries.delete(emoteId);
      }
    };
  };

  const subscriberCount = (emoteId: string): number =>
    entries.get(emoteId)?.listeners.size ?? 0;

  return { subscribe, subscriberCount };
}

/** The retry the chat rows share. */
export const emoteImageRetry = createEmoteImageRetry();
