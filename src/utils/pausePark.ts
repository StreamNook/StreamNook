/**
 * What a live player does with a pause. A paused hls.js keeps polling the
 * playlist, downloading parts and re-seeking the paused element to live, so
 * two seconds into a pause loading stops (the phones' rule,
 * mobile/player/useMobileHlsEngine.ts). A play within the live window resumes
 * loading at the paused frame; past it loading resumes too and the caller
 * moves the playhead to the live edge (utils/liveResync.ts).
 *
 * Pause intents outlive a player instance: a restart, an ad pivot or a quality
 * change rebuilds the player, and the rebuilt one must stay paused.
 */

/** A live pause this long stops the player's downloads. */
export const STOP_LOAD_AFTER_PAUSE_MS = 2000;

/** A pause this long resumes at the live edge instead of the paused frame. The
 *  relay's low-latency origin parks after the same span without a playlist
 *  request (ll_origin.rs PARK_IDLE_MS). */
export const RESYNC_AFTER_PAUSE_MS = 30_000;

/** localStorage key a test run sets to shorten the spans, for example
 *  `{"stopLoadMs": 2000, "resyncMs": 5000}`. Positive numbers only. */
export const PAUSE_PARK_OVERRIDE_KEY = 'streamnook.dev.pause-park';

export interface PauseParkTimings {
  stopLoadMs: number;
  resyncMs: number;
}

/** The spans in force: the defaults, or a test run's override. */
export function pauseParkTimings(): PauseParkTimings {
  const timings: PauseParkTimings = { stopLoadMs: STOP_LOAD_AFTER_PAUSE_MS, resyncMs: RESYNC_AFTER_PAUSE_MS };
  try {
    const raw = typeof localStorage === 'undefined' ? null : localStorage.getItem(PAUSE_PARK_OVERRIDE_KEY);
    if (!raw) return timings;
    const o = JSON.parse(raw) as Partial<PauseParkTimings>;
    if (typeof o.stopLoadMs === 'number' && o.stopLoadMs > 0) timings.stopLoadMs = o.stopLoadMs;
    if (typeof o.resyncMs === 'number' && o.resyncMs > 0) timings.resyncMs = o.resyncMs;
  } catch {
    /* storage blocked or not JSON: the defaults */
  }
  return timings;
}

/**
 * The resync cut-off for a player whose origin serves `totalDuration` seconds
 * of playlist while the element holds `forwardBuffer` seconds ahead of the
 * paused frame: the pause outlives the window once everything past the buffer
 * has rolled off the playlist. Never more than `cap`, never negative. With no
 * playlist details it is `cap`.
 */
export function resyncCutoffMs(
  totalDuration: number | null | undefined,
  forwardBuffer: number,
  cap = RESYNC_AFTER_PAUSE_MS,
): number {
  if (totalDuration == null || !Number.isFinite(totalDuration)) return cap;
  const fwd = Number.isFinite(forwardBuffer) && forwardBuffer > 0 ? forwardBuffer : 0;
  return Math.min(cap, Math.max(0, (totalDuration - fwd) * 1000));
}

/** The pause spans for a player: the defaults (or a test run's override) with
 *  the resync cut-off cut down to what its playlist window and buffer allow. */
export function windowedPauseTimings(
  hls: { latestLevelDetails?: { totalduration?: number } | null },
  video: Pick<HTMLVideoElement, 'buffered' | 'currentTime'>,
): PauseParkTimings {
  const base = pauseParkTimings();
  let forward = 0;
  const b = video.buffered;
  for (let i = 0; i < b.length; i++) {
    if (b.start(i) <= video.currentTime + 0.05 && b.end(i) >= video.currentTime) {
      forward = b.end(i) - video.currentTime;
      break;
    }
  }
  return { ...base, resyncMs: resyncCutoffMs(hls.latestLevelDetails?.totalduration, forward, base.resyncMs) };
}

export type PlayOutcome ='none' | 'resume' | 'resync';

export interface PausePark {
  /** The video paused (or a rebuilt player holds an earlier pause made at `at`). */
  paused(at?: number): void;
  /** The video is playing again. 'resume': loading restarted at the paused
   *  frame. 'resync': the pause outlived the live window; loading restarted if
   *  it had stopped, and the caller moves the playhead to the edge. 'none':
   *  nothing to do. */
  play(now?: number): PlayOutcome;
  /** The player is going away. */
  dispose(): void;
  readonly loadStopped: boolean;
}

export interface PauseParkOptions {
  stopLoad: () => void;
  startLoad: () => void;
  /** True while this instance is still current and its video still paused. */
  stillPaused: () => boolean;
  log?: (message: string) => void;
  now?: () => number;
  timings?: () => PauseParkTimings;
}

export function createPausePark(o: PauseParkOptions): PausePark {
  const now = o.now ?? Date.now;
  const timings = o.timings ?? pauseParkTimings;
  let pausedAt: number | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let stopped = false;
  let disposed = false;
  const clear = () => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
  };
  return {
    get loadStopped() {
      return stopped;
    },
    paused(at = now()) {
      if (disposed) return;
      if (pausedAt === null || at < pausedAt) pausedAt = at;
      if (stopped || timer !== null) return;
      const wait = Math.max(0, pausedAt + timings().stopLoadMs - now());
      timer = setTimeout(() => {
        timer = null;
        if (disposed || stopped || !o.stillPaused()) return;
        try {
          o.stopLoad();
          stopped = true;
          o.log?.('paused; downloads stopped until play');
        } catch {
          /* player already torn down */
        }
      }, wait);
    },
    play(at = now()) {
      if (disposed) return 'none';
      clear();
      const since = pausedAt;
      pausedAt = null;
      if (since === null) return 'none';
      const wasStopped = stopped;
      if (wasStopped) {
        stopped = false;
        try {
          o.startLoad();
        } catch {
          /* player already torn down */
        }
      }
      if (at - since >= timings().resyncMs) return 'resync';
      return wasStopped ? 'resume' : 'none';
    },
    dispose() {
      disposed = true;
      clear();
    },
  };
}

export interface PauseIntent {
  key: string;
  at: number;
}

const intents = new Map<string, PauseIntent>();

/** The viewer paused `key` (a channel key) in player `scope` ('main' or a tile id). */
export function notePauseIntent(scope: string, key: string, at = Date.now()): void {
  const cur = intents.get(scope);
  if (!cur || cur.key !== key) intents.set(scope, { key, at });
}

export function clearPauseIntent(scope: string): void {
  intents.delete(scope);
}

/** The pause to keep for `key` in `scope`, or null. A pause for another key is
 *  dropped here: a different channel always plays. A null key (nothing on
 *  screen) reads nothing and drops nothing. */
export function heldPause(scope: string, key: string | null): PauseIntent | null {
  const cur = intents.get(scope);
  if (!cur || key === null) return null;
  if (cur.key === key) return cur;
  intents.delete(scope);
  return null;
}

/** The pause recorded in `scope`, whatever its channel, without changing it. */
export function pauseIntentOf(scope: string): PauseIntent | null {
  return intents.get(scope) ?? null;
}

/** The channel the own-channel pause was last applied to, so a restart that
 *  starts the same channel again does not pause it a second time after the
 *  viewer pressed play. */
let ownChannelHeld: string | null = null;

/** The signed-in account opened its own live channel (it is the broadcaster):
 *  the main player opens paused, chat only, once per opening. True when the
 *  pause was applied. */
export function holdOwnChannel(key: string, at = Date.now()): boolean {
  if (ownChannelHeld === key) return false;
  ownChannelHeld = key;
  notePauseIntent('main', key, at);
  return true;
}

/** The channel `key` is already open as the viewer's own (a restart brings
 *  back a stream they were watching): its next opening does not pause it. */
export function markOwnChannelOpened(key: string): void {
  ownChannelHeld = key;
}

/** Another channel opened, or the stream stopped: the next opening of the
 *  broadcaster's own channel pauses again. */
export function forgetOwnChannel(): void {
  ownChannelHeld = null;
}
