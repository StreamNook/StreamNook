// "The viewer is using this window" (SN-284). A pinned mention's time counts
// only while someone is using StreamNook, and Rust cannot see input in the
// page, so the page tells it: `note_viewer_input` (no arguments; Rust stamps
// its own receipt time), on the leading and the trailing edge, at most once a
// second. The listeners are window-level, capture-phase and passive, and do
// nothing but call `touch`, so they cost no render and never block input.
// They are installed only while Pin mentions is on.

import { invoke } from '@tauri-apps/api/core';
import { Logger } from './logger';

export const INPUT_GAP_MS = 1000;

/** What counts as use: pointer, wheel, keys, touch and typing in the composer. */
export const INPUT_EVENTS = ['pointerdown', 'pointermove', 'wheel', 'keydown', 'touchstart', 'input'] as const;

export interface InputNotifierDeps {
  send: () => void;
  now?: () => number;
  schedule?: (fn: () => void, ms: number) => unknown;
  cancel?: (handle: unknown) => void;
  gapMs?: number;
}

export function createInputNotifier(deps: InputNotifierDeps) {
  const now = deps.now ?? Date.now;
  const schedule = deps.schedule ?? ((fn, ms) => setTimeout(fn, ms));
  const cancel = deps.cancel ?? ((h) => clearTimeout(h as ReturnType<typeof setTimeout>));
  const gap = deps.gapMs ?? INPUT_GAP_MS;
  let lastSent = -Infinity;
  let pending: unknown = null;

  const fire = () => {
    pending = null;
    lastSent = now();
    deps.send();
  };

  return {
    /** One input happened. The leading edge sends now; inside the gap one
     *  trailing send is queued for when the gap ends. */
    touch(): void {
      if (pending !== null) return;
      const since = now() - lastSent;
      if (since >= gap) fire();
      else pending = schedule(fire, gap - since);
    },
    dispose(): void {
      if (pending !== null) cancel(pending);
      pending = null;
    },
  };
}

let notifier: ReturnType<typeof createInputNotifier> | null = null;
let installed = false;
let onSend: (() => void) | null = null;

/** Called on every throttled send (the phones retry a failed first
 *  `set_app_interactive` report here). */
export function setInputSideEffect(fn: (() => void) | null): void {
  onSend = fn;
}

const touch = () => notifier?.touch();

/** Installs or removes the window listeners. Idempotent. */
export function setPinInputEnabled(on: boolean): void {
  if (on === installed) return;
  installed = on;
  if (on) {
    notifier = createInputNotifier({
      send: () => {
        invoke('note_viewer_input').catch((err) => Logger.debug('[PinInput] note_viewer_input failed:', err));
        try {
          onSend?.();
        } catch {
          /* a side effect never blocks the report */
        }
      },
    });
    for (const name of INPUT_EVENTS) window.addEventListener(name, touch, { capture: true, passive: true });
  } else {
    for (const name of INPUT_EVENTS) window.removeEventListener(name, touch, { capture: true });
    notifier?.dispose();
    notifier = null;
  }
}
