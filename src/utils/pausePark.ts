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
 *  relay's low-latency origin parks only after a longer span without a playlist
 *  request (ll_origin.rs PARK_IDLE_MS), so a player on it asks the origin to
 *  rebuild its window before it loads again (`resyncLoad`). */
export const RESYNC_AFTER_PAUSE_MS = 30_000;

/** Seconds of playlist window kept in hand when a play resumes at the paused
 *  frame, for a playlist that reports no target duration. */
export const RESUME_MARGIN_FALLBACK_SEC = 4;

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
 * The resync cut-off for a player whose buffer around the paused frame ends at
 * `bufferEnd` while the origin's playlist now begins at `windowStart` (`age`
 * seconds old): the pause outlives the window once the playlist's start has
 * rolled past the end of the buffer, so nothing after the buffer can be
 * fetched. `marginSec` is cut off the room that is left: a play restarts
 * loading on a playlist reload and the next fragment's fetch, which land a
 * target duration or more after the play, by which time the window has moved
 * on. Never more than `cap`, never negative. With no playlist details it is
 * `cap`.
 */
export function resyncCutoffMs(
  bufferEnd: number,
  windowStart: number | null | undefined,
  age: number | null | undefined,
  cap = RESYNC_AFTER_PAUSE_MS,
  marginSec = 0,
): number {
  if (windowStart == null || !Number.isFinite(windowStart) || !Number.isFinite(bufferEnd)) return cap;
  const aged = age != null && Number.isFinite(age) && age > 0 ? age : 0;
  return Math.min(cap, Math.max(0, (bufferEnd - windowStart - aged - marginSec) * 1000));
}

/** The pause spans for a player: the defaults (or a test run's override) with
 *  the resync cut-off cut down to what its playlist window and buffer allow,
 *  measured now. The margin is two target durations, or
 *  `RESUME_MARGIN_FALLBACK_SEC` when the playlist reports none. */
export function windowedPauseTimings(
  hls: { latestLevelDetails?: { fragmentStart?: number; age?: number; targetduration?: number } | null },
  video: Pick<HTMLVideoElement, 'buffered' | 'currentTime'>,
): PauseParkTimings {
  const base = pauseParkTimings();
  let bufferEnd = video.currentTime;
  const b = video.buffered;
  for (let i = 0; i < b.length; i++) {
    if (b.start(i) <= video.currentTime + 0.05 && b.end(i) >= video.currentTime) {
      bufferEnd = b.end(i);
      break;
    }
  }
  const details = hls.latestLevelDetails;
  const target = details?.targetduration;
  const marginSec = target != null && Number.isFinite(target) && target > 0 ? 2 * target : RESUME_MARGIN_FALLBACK_SEC;
  return {
    ...base,
    resyncMs: resyncCutoffMs(bufferEnd, details?.fragmentStart, details?.age, base.resyncMs, marginSec),
  };
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
  /** Loads again after a pause past the resync cut-off, in place of
   *  `startLoad`. It owns the restart: the park stops any load still running
   *  first and never calls `startLoad` for that play. */
  resyncLoad?: () => void;
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
  // The resync cut-off, measured when the pause starts: the playlist window
  // keeps moving while paused, so a reading at play time would be too late.
  let resyncMs = 0;
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
      if (pausedAt === null) resyncMs = timings().resyncMs;
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
      if (at - since >= resyncMs && o.resyncLoad) {
        try {
          if (!stopped) o.stopLoad();
          o.resyncLoad();
        } catch {
          /* player already torn down */
        }
        stopped = false;
        return 'resync';
      }
      const wasStopped = stopped;
      if (wasStopped) {
        stopped = false;
        try {
          o.startLoad();
        } catch {
          /* player already torn down */
        }
      }
      if (at - since >= resyncMs) return 'resync';
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

/** Tiles (by slot id) whose own-channel pause was already applied. Module
 *  level, so a tile that docks and undocks (its component remounts) is not
 *  paused a second time after the viewer pressed play. */
const heldTiles = new Set<string>();

/** The own-channel pause was applied to tile `id`. */
export function markTileHeld(id: string): void {
  heldTiles.add(id);
}

/** Whether tile `id` already had its own-channel pause applied. */
export function isTileHeld(id: string): boolean {
  return heldTiles.has(id);
}

/** Tile `id` was removed: its next opening pauses again. */
export function forgetTile(id: string): void {
  heldTiles.delete(id);
}

/** Tile `id`'s stream ended: its pause and its own-channel hold go, so the
 *  next opening of the channel plays (or pauses, if it is the viewer's own)
 *  afresh. */
export function clearTileOnEnd(id: string): void {
  clearPauseIntent(id);
  forgetTile(id);
}

/** Whether a tile's stream has ended, as its slot reads when its player goes
 *  away: the offline edge marks the slot offline and failed together, and a
 *  retry clears the failure, so a tile that is only restarting does not count. */
export function tileStreamEnded(slot: { offline?: boolean; loadError?: boolean } | null | undefined): boolean {
  return !!slot?.offline && !!slot.loadError;
}

/** Tiles (slot ids) that hold a live pause, for a snapshot. */
export function pausedTileIds(): string[] {
  const ids: string[] = [];
  for (const [scope, intent] of intents) {
    if (scope !== 'main' && intent.key === scope) ids.push(scope);
  }
  return ids;
}

/** Tiles (slot ids) whose own-channel pause was applied, for a snapshot. */
export function heldTileIds(): string[] {
  return [...heldTiles];
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
