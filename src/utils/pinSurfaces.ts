// The pin surfaces one window shows (SN-284). Rust decides where a pinned
// mention lives and when its clock runs; it can only do that if it knows which
// chat stacks this window has mounted, which of them are on screen and which
// pins are in view. This keeps that map and sends it in one debounced call.
//
// Rules (PLAN 4a/4b):
// - `page` is this document (`performance.timeOrigin`); `seq` orders its calls
//   and survives a hot reload (`import.meta.hot.data`). Rust drops older ones.
// - Every mount has its own token; an unmount removes only its own entry, so a
//   StrictMode double mount or a remount of one id never removes the new one.
// - Nothing is sent while Pin mentions is off. Turning it on sends the whole
//   map at once, and the page-start call is the first one (seq 0).
// - A surface that is not `shown` reports no visible pins.

import { invoke } from '@tauri-apps/api/core';
import { Logger } from './logger';

export interface PinRoomRef {
  provider: string;
  channel: string;
}

/** What one mounted stack says about itself (Rust `SurfaceIn`, minus the token). */
export interface PinSurfaceSpec {
  /** `main-chat`, `dock`, `pane:<leaf id>`, `pane:<entryKey>`, `main-provider`,
   *  `blended`, `phone-watch`, `phone-chats`. */
  id: string;
  rooms: PinRoomRef[];
  /** The surface the viewer is acting in (the focused pane, the visible tab). */
  active: boolean;
  /** Drawn and not covered by the app's own UI (`chatSurfaceShown`). */
  shown: boolean;
  /** Phone watch screen: `stacked`, `landscape`, `vertical`, `over-video`. */
  mode: string;
  strips_only: boolean;
  over_video: boolean;
  /** The platforms a tap on a pin here can open. */
  opens: string[];
  /** Every pin id in view now (empty whenever not shown). */
  visible: string[];
}

export interface PinSurfaceOut extends PinSurfaceSpec {
  token: string;
}

export interface PinSurfaceCall {
  page: string;
  seq: number;
  surfaces: PinSurfaceOut[];
}

export interface PinSurfaceRegistryDeps {
  send: (call: PinSurfaceCall) => Promise<unknown> | void;
  page: string;
  /** Where the last used seq lives across a hot reload. */
  seqStore?: { get: () => number | undefined; set: (n: number) => void };
  debounceMs?: number;
  schedule?: (fn: () => void, ms: number) => unknown;
  cancel?: (handle: unknown) => void;
}

/** The most visible ids one surface reports (Rust truncates at the same size). */
const VISIBLE_MAX = 64;

const same = (a: unknown, b: unknown): boolean => JSON.stringify(a) === JSON.stringify(b);

export function createPinSurfaceRegistry(deps: PinSurfaceRegistryDeps) {
  const debounceMs = deps.debounceMs ?? 50;
  const schedule = deps.schedule ?? ((fn, ms) => setTimeout(fn, ms));
  const cancel = deps.cancel ?? ((h) => clearTimeout(h as ReturnType<typeof setTimeout>));
  const surfaces = new Map<string, PinSurfaceSpec>();
  let enabled = false;
  let timer: unknown = null;
  let seq = deps.seqStore?.get() ?? -1;

  const snapshot = (): PinSurfaceOut[] =>
    [...surfaces.entries()].map(([token, s]) => ({
      ...s,
      token,
      // Nothing is in view on a surface that is not shown.
      visible: s.shown ? s.visible.slice(0, VISIBLE_MAX) : [],
    }));

  const sendNow = () => {
    if (timer !== null) {
      cancel(timer);
      timer = null;
    }
    if (!enabled) return;
    seq += 1;
    deps.seqStore?.set(seq);
    const res = deps.send({ page: deps.page, seq, surfaces: snapshot() });
    if (res && typeof (res as Promise<unknown>).catch === 'function') {
      (res as Promise<unknown>).catch((err) => Logger.debug('[PinSurfaces] set_pin_surfaces failed:', err));
    }
  };

  const queue = () => {
    if (!enabled) return;
    if (timer !== null) cancel(timer);
    timer = schedule(sendNow, debounceMs);
  };

  return {
    register(token: string, spec: PinSurfaceSpec): void {
      surfaces.set(token, spec);
      queue();
    },
    /** Merge changes into one mount's entry; sends only when something changed. */
    update(token: string, patch: Partial<PinSurfaceSpec>): void {
      const prev = surfaces.get(token);
      if (!prev) return;
      const next = { ...prev, ...patch };
      if (same(prev, next)) return;
      surfaces.set(token, next);
      queue();
    },
    /** Removes this mount's entry only. */
    unregister(token: string): void {
      if (surfaces.delete(token)) queue();
    },
    /** Pin mentions on or off. Turning on sends the whole map at once (the
     *  first call of a page is its page-start); off sends nothing. */
    setEnabled(on: boolean): void {
      if (on === enabled) return;
      enabled = on;
      if (!on) {
        if (timer !== null) {
          cancel(timer);
          timer = null;
        }
        return;
      }
      sendNow();
    },
    /** Send the map again (focus, visibility, `mention-pins-changed`): the
     *  in-view sets are reported on change only, so Rust may have dropped one
     *  while the window was away. */
    resend(): void {
      queue();
    },
    isEnabled: () => enabled,
    size: () => surfaces.size,
    /** Test and debug read. */
    peek: () => snapshot(),
  };
}

// ---------------------------------------------------------------------------
// This window's registry
// ---------------------------------------------------------------------------

const hotData = (import.meta as unknown as { hot?: { data: Record<string, unknown> } }).hot?.data;

export const pinSurfaces = createPinSurfaceRegistry({
  page: String(performance.timeOrigin),
  seqStore: {
    get: () => (typeof hotData?.pinSeq === 'number' ? (hotData.pinSeq as number) : undefined),
    set: (n) => {
      if (hotData) hotData.pinSeq = n;
    },
  },
  send: (call) => invoke('set_pin_surfaces', { page: call.page, seq: call.seq, surfaces: call.surfaces }),
});
