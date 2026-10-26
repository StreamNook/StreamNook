// The paused-stream rules, as pure functions the player, the lock-screen
// session and their tests share. No DOM, no timers of its own.

/** Paused this long, resuming jumps back to live (Rust parks the relay at the
 *  same mark: earning_gate::PARK_AFTER). */
export const RESYNC_AFTER_PAUSE_MS = 30_000;
/** Paused AND backgrounded this long, the media card and its service stand down. */
export const STAND_DOWN_AFTER_MS = 10 * 60_000;
// Checked against the WALL clock once a minute rather than one 10-minute timer:
// Chromium's timers run on a monotonic clock that stops in deep sleep, so a
// single long timer could sit for hours on a sleeping phone. A minute check
// catches up on the first wake.
const STAND_DOWN_CHECK_MS = 60_000;

export function resyncOnResume(pausedAt: number | null, now: number): boolean {
  return pausedAt !== null && now - pausedAt >= RESYNC_AFTER_PAUSE_MS;
}

export type SurfaceLossAction = 'none' | 'downshift' | 'pause';

/** The player lost its surface (screen off, or hidden without PiP). Background
 *  play on keeps the audio (audio rendition); off pauses. PiP is a surface. */
export function surfaceLossAction(o: { inPip: boolean; playing: boolean; backgroundPlay: boolean }): SurfaceLossAction {
  if (o.inPip || !o.playing) return 'none';
  return o.backgroundPlay ? 'downshift' : 'pause';
}

export interface StandDownDeps {
  now(): number;
  schedule(fn: () => void, ms: number): () => void;
  onDue(): void;
}

export function createStandDownWatch(deps: StandDownDeps) {
  let since: number | null = null;
  let cancel: (() => void) | null = null;
  const check = () => {
    cancel = null;
    if (since === null) return;
    if (deps.now() - since >= STAND_DOWN_AFTER_MS) {
      since = null;
      deps.onDue();
      return;
    }
    cancel = deps.schedule(check, STAND_DOWN_CHECK_MS);
  };
  return {
    arm(): void {
      if (since !== null) return;
      since = deps.now();
      cancel = deps.schedule(check, STAND_DOWN_CHECK_MS);
    },
    disarm(): void {
      since = null;
      cancel?.();
      cancel = null;
    },
    get armed(): boolean {
      return since !== null;
    },
  };
}
