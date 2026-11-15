// What the page tells Rust about a notice click or a boot-time action, so a link
// that arrived earlier does not open over something the person chose later.
//
// Rust orders by one counter: every stored link and every notice click takes a
// stamp. The page reports only what it actually opened, and Rust drops only a
// link stamped before that action. Plain TypeScript with injected deps, so a
// vitest can drive every case.

import { isMainWindow } from './platform';
import type { NoticeAction } from './notificationMirror';

/** The one shape of Tauri's `invoke` this file needs. */
export type InvokeFn = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

/** How long notice work may hold the first take after boot. */
const NOTICE_WORK_LIMIT_MS = 3000;

/**
 * A notice's page action ran from a click on a notice the page holds a closure
 * for. Reports only when a closure was found: a dead click opens nothing, so it
 * must not clear a good link.
 */
export function reportActivated(
  payload: { id: string; seq: number },
  actions: Map<string, () => void>,
  report: (seq: number) => void,
): void {
  const run = actions.get(payload.id);
  actions.delete(payload.id);
  if (run) {
    run();
    report(payload.seq);
  }
}

/**
 * A notice action taken from Rust after a restart. `run` calls `opened` right
 * where it opens something; the report follows that call, never the click.
 * Returns what `run` returns, so a deferred open stays in the tracked work.
 */
export function reportTaken(
  taken: { action: NoticeAction; seq: number } | null,
  run: (action: NoticeAction, opened: () => void) => void | Promise<void>,
  report: (seq: number) => void,
): void | Promise<void> {
  if (taken) return run(taken.action, () => report(taken.seq));
}

// Each entry is a gate that opens when its work settles or when its own limit
// is up, whichever comes first, so work that never settles stops holding later
// takes once its own 3 s are over.
const work = new Set<Promise<void>>();

/** Track `p` so the first take after boot waits for it. Returns `p` itself. */
export function trackNoticeWork<T>(p: Promise<T>): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const gate: Promise<void> = new Promise<void>((open) => {
    const leave = () => {
      clearTimeout(timer);
      work.delete(gate);
      open();
    };
    timer = setTimeout(leave, NOTICE_WORK_LIMIT_MS);
    // Both arms: a rejection must not become an unhandled one of its own.
    p.then(leave, leave);
  });
  work.add(gate);
  return p;
}

/**
 * Resolves when every tracked entry is done, including entries a finished one
 * added (a take that runs an action that reports). The whole wait is bounded by
 * one 3 s timer and resolves anyway when it wins.
 */
export async function noticeWorkSettled(): Promise<void> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const limit = new Promise<void>((done) => {
    timer = setTimeout(done, NOTICE_WORK_LIMIT_MS);
  });
  const all = (async () => {
    const waited = new Set<Promise<void>>();
    for (;;) {
      const fresh = [...work].filter((gate) => !waited.has(gate));
      if (fresh.length === 0) return;
      fresh.forEach((gate) => waited.add(gate));
      await Promise.all(fresh);
    }
  })();
  try {
    await Promise.race([all, limit]);
  } finally {
    clearTimeout(timer);
  }
}

// The highest link stamp the page has heard of (events and peeks), 0 for none.
let lastSeen = 0;
// The peek that was started at mount and has not settled yet.
let pendingPeek: Promise<unknown> | null = null;
// True while an automatic write runs: not a person's action.
let automatic = false;

/** Keep the highest stamp the page has seen. */
export function noteLinkStamp(stamp: number): void {
  if (Number.isFinite(stamp) && stamp > lastSeen) lastSeen = stamp;
}

/** The highest stamp seen, 0 when no link has been heard of. */
export function lastSeenLinkStamp(): number {
  return lastSeen;
}

/** Keep `p` as the pending mount peek, track it, and return it. */
export function trackLinkPeek<T>(p: Promise<T>): Promise<T> {
  pendingPeek = p;
  const clear = () => {
    if (pendingPeek === p) pendingPeek = null;
  };
  p.then(clear, clear);
  return trackNoticeWork(p);
}

/**
 * Run `fn` as an automatic write (a restore, a default choice): a change made
 * inside it is not an action. Nesting keeps the outer flag, and a throwing `fn`
 * does not leave it set.
 */
export function runAsAutomatic<T>(fn: () => T): T {
  const prev = automatic;
  automatic = true;
  try {
    return fn();
  } finally {
    automatic = prev;
  }
}

/** True while `runAsAutomatic` is running. */
export function isAutomatic(): boolean {
  return automatic;
}

/**
 * A person did something that opens a page or a stream while the app boots.
 * Tell Rust which link the page had heard of, so only that older link is
 * dropped. The step is tracked, so the first take after boot waits for it.
 */
export function noteBootAction(invoke: InvokeFn): void {
  if (automatic) return;
  void trackNoticeWork(
    (async () => {
      let seen = lastSeen;
      if (seen === 0 && pendingPeek) {
        // The mount peek has not answered yet: wait for it, whatever it does.
        await pendingPeek.catch(() => undefined);
        seen = lastSeen;
      }
      if (seen === 0) return;
      await invoke('link_superseded', { seen }).catch(() => {});
    })(),
  );
}

/** The fields of the app store that decide whether a change opened a page. */
export interface PageWatchState {
  isBooting?: boolean;
  isSettingsOpen?: boolean;
  showBadgesOverlay?: boolean;
  showWhispersOverlay?: boolean;
  showEmoteSetsOverlay?: boolean;
  showDropsOverlay?: boolean;
  showLiveStreamsOverlay?: boolean;
  showMarketplaceOverlay?: boolean;
  showDashboardOverlay?: boolean;
  isCommandPaletteOpen?: boolean;
  profileViewerUserId?: string | null;
  songsPanel?: unknown;
  emoteSpotlight?: unknown;
  profileModalUser?: unknown;
  clipModal?: unknown;
  vodModal?: unknown;
  homeActiveTab?: string;
  homeSelectedCategory?: { id: string } | null;
  homeProviderCategory?: { id: string } | null;
  activePlatform?: string;
}

const OPENED_FLAGS = [
  'isSettingsOpen',
  'showBadgesOverlay',
  'showWhispersOverlay',
  'showEmoteSetsOverlay',
  'showDropsOverlay',
  'showLiveStreamsOverlay',
  'showMarketplaceOverlay',
  'showDashboardOverlay',
  'isCommandPaletteOpen',
] as const;

const OPENED_SLOTS = [
  'profileViewerUserId',
  'songsPanel',
  'emoteSpotlight',
  'profileModalUser',
  'clipModal',
  'vodModal',
] as const;

/**
 * True when a layer held by the app store went from closed to open, or Home's
 * page changed (its tab, its category, its provider category), or the platform
 * scope changed. A sidebar toggle, a volume change or a theme change is not a
 * page. A layer another store holds is not here: that store's open path calls
 * `noteBootAction` itself.
 */
export function opensPageOverPlayer(prev: PageWatchState, next: PageWatchState): boolean {
  if (OPENED_FLAGS.some((flag) => !prev[flag] && !!next[flag])) return true;
  if (OPENED_SLOTS.some((slot) => prev[slot] == null && next[slot] != null)) return true;
  return (
    prev.homeActiveTab !== next.homeActiveTab ||
    prev.homeSelectedCategory?.id !== next.homeSelectedCategory?.id ||
    prev.homeProviderCategory?.id !== next.homeProviderCategory?.id ||
    prev.activePlatform !== next.activePlatform
  );
}

/**
 * Watch the store while the app boots: a change that opens a page, made by a
 * person in the main window, supersedes the links the page has heard of. The
 * caller stops it when boot ends, so it never runs after boot.
 */
export function watchBootActions<S extends PageWatchState>(
  store: {
    getState: () => S;
    subscribe: (listener: (state: S, prev: S) => void) => () => void;
  },
  invoke: InvokeFn,
): () => void {
  return store.subscribe((state, prev) => {
    if (!store.getState().isBooting || !isMainWindow()) return;
    if (opensPageOverPlayer(prev, state)) noteBootAction(invoke);
  });
}

/** For tests: forget every tracked entry, stamp, peek and flag. */
export function resetNoticeReportForTest(): void {
  work.clear();
  lastSeen = 0;
  pendingPeek = null;
  automatic = false;
}
