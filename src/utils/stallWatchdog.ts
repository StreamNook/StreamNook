export interface StallWatchOptions {
  /** The playhead stood still while the element was trying to play. */
  onFrozen: () => void;
  /** How long a still playhead is tolerated before `onFrozen`. */
  frozenMs?: number;
  /** How long after `onFrozen` before it can run again. */
  cooldownMs?: number;
  now?: () => number;
}

/** How often the playhead is sampled between timeupdate events. */
const SAMPLE_MS = 1000;

/** Seconds the playhead must move to count as advancing. */
const ADVANCE_SECS = 0.05;

/** The element knows its media (HTMLMediaElement.HAVE_METADATA). */
const HAVE_METADATA = 1;

/** The element can show the frame at its playhead (HTMLMediaElement.HAVE_CURRENT_DATA). */
const HAVE_CURRENT_DATA = 2;

/** Buffered seconds ahead of the playhead that rule out a plain shortage. */
const WEDGE_AHEAD_SECS = 1;

/**
 * WebKit can wedge a media element at the start of a stream: it is playing,
 * the playhead sits inside a buffered range with media ahead of it, and the
 * readyState still never climbs past HAVE_METADATA. Every seek on it then
 * stays pending (`seeking` true for good), so the stall checks that skip a
 * seeking element never see it. Only a fresh load gets it playing again.
 */
export function wedgedOnBuffer(
  video: Pick<HTMLMediaElement, 'paused' | 'readyState' | 'currentTime' | 'buffered'>,
): boolean {
  if (video.paused || video.readyState >= HAVE_CURRENT_DATA) return false;
  const b = video.buffered;
  const t = video.currentTime;
  for (let i = 0; i < b.length; i++) {
    if (b.start(i) <= t && b.end(i) - t >= WEDGE_AHEAD_SECS) return true;
  }
  return false;
}

/**
 * Reports a playhead that stays pinned while the element is playing: not
 * paused, not seeking and holding media. A pause, a seek or a move of the
 * playhead starts the count again, and after a report the next one waits for
 * the cooldown. Returns the stop, which releases the interval and the element
 * listeners.
 */
export function watchStalls(
  video: HTMLVideoElement,
  { onFrozen, frozenMs = 8000, cooldownMs = 15000, now = Date.now }: StallWatchOptions,
): () => void {
  let anchor = video.currentTime;
  let anchoredAt = now();
  let firedAt = -Infinity;
  const reset = () => {
    anchor = video.currentTime;
    anchoredAt = now();
  };
  const sample = () => {
    if (video.paused || video.seeking || video.readyState < HAVE_METADATA) {
      reset();
      return;
    }
    if (Math.abs(video.currentTime - anchor) > ADVANCE_SECS) {
      reset();
      return;
    }
    const t = now();
    if (t - anchoredAt < frozenMs || t - firedAt < cooldownMs) return;
    firedAt = t;
    onFrozen();
  };
  const interval = setInterval(sample, SAMPLE_MS);
  video.addEventListener('timeupdate', sample);
  video.addEventListener('pause', reset);
  video.addEventListener('seeking', reset);
  return () => {
    clearInterval(interval);
    video.removeEventListener('timeupdate', sample);
    video.removeEventListener('pause', reset);
    video.removeEventListener('seeking', reset);
  };
}

/** How long an element must sit wedged with a still playhead before a reload. */
export const WEDGE_RELOAD_MS = 4000;

/** What one watchdog tick learned about a wedge. */
export interface WedgeTick {
  /** The element is wedged now: the caller skips its other stall checks. */
  wedged: boolean;
  /** The wedge has lasted `WEDGE_RELOAD_MS`: reload the stream. */
  reload: boolean;
}

type WedgeMedia = Pick<HTMLMediaElement, 'paused' | 'readyState' | 'currentTime' | 'buffered'>;

/**
 * Times a wedge (`wedgedOnBuffer`) across watchdog ticks by the clock, not by
 * counting ticks, so it never reloads before `holdMs` however the ticks fall.
 * The playhead must stay still too: an element still decoding (a covered
 * window, for one) moves it and is timed afresh. After a reload the timing
 * starts over.
 */
export function watchWedge(holdMs = WEDGE_RELOAD_MS): (video: WedgeMedia, now: number) => WedgeTick {
  let since: number | null = null;
  let sinceCt = 0;
  return (video, now) => {
    if (!wedgedOnBuffer(video)) {
      since = null;
      return { wedged: false, reload: false };
    }
    if (since === null || Math.abs(video.currentTime - sinceCt) > ADVANCE_SECS) {
      since = now;
      sinceCt = video.currentTime;
      return { wedged: true, reload: false };
    }
    if (now - since < holdMs) return { wedged: true, reload: false };
    since = null;
    return { wedged: true, reload: true };
  };
}

/** How often a start that has its level details checks for its first media. */
export const FIRST_MEDIA_CHECK_MS = 1000;
/** Nothing in flight and nothing loaded for this long: the start is broken. */
export const FIRST_MEDIA_IDLE_MS = 5000;
/** The longest a start waits for its first media: the fragment load's own timeout. */
export const FIRST_MEDIA_CAP_MS = 20000;

export interface FirstMediaDeps {
  now: () => number;
  /** Run `fn` after `ms`; the caller keeps the handle so its teardown can clear it. */
  schedule: (fn: () => void, ms: number) => void;
  /** This player is still the one playing. */
  isCurrent: () => boolean;
  /** Reload the stream; false when the reload was refused for now. */
  reload: (why: string) => boolean;
  /** Called once when the wait ends, however it ends. */
  finished: () => void;
}

export interface FirstMediaWatch {
  /** A fragment or part started downloading. */
  loading: () => void;
  /** A fragment or part finished downloading. */
  loaded: () => void;
  /** A fragment or part was parsed. */
  progress: () => void;
  /** The first media was buffered: true once, when the caller should start. */
  media: () => boolean;
}

/**
 * Waits for a start's first buffered media and reloads the stream when it
 * never comes: after `FIRST_MEDIA_IDLE_MS` with nothing in flight and nothing
 * loaded, or `FIRST_MEDIA_CAP_MS` at the latest, so a slow download still in
 * flight is never thrown away. A refused reload (a restart or an auto-switch
 * is running) is asked again on the next check.
 */
export function watchFirstMedia(deps: FirstMediaDeps): FirstMediaWatch {
  const armedAt = deps.now();
  let lastProgressAt = armedAt;
  let inFlight = false;
  let done = false;
  const finish = () => {
    if (done) return;
    done = true;
    deps.finished();
  };
  const check = () => {
    if (done) return;
    if (!deps.isCurrent()) {
      finish();
      return;
    }
    const now = deps.now();
    const waited = now - armedAt;
    const capped = waited >= FIRST_MEDIA_CAP_MS;
    const idle = !inFlight && now - lastProgressAt >= FIRST_MEDIA_IDLE_MS;
    if (capped || idle) {
      const why = `[HLS] low-latency start: no media buffered after ${Math.round(waited / 1000)}s (${capped ? 'cap' : 'idle'}); reloading the stream`;
      if (deps.reload(why)) {
        finish();
        return;
      }
    }
    deps.schedule(check, FIRST_MEDIA_CHECK_MS);
  };
  deps.schedule(check, FIRST_MEDIA_CHECK_MS);
  return {
    loading: () => {
      inFlight = true;
      lastProgressAt = deps.now();
    },
    loaded: () => {
      inFlight = false;
      lastProgressAt = deps.now();
    },
    progress: () => {
      lastProgressAt = deps.now();
    },
    media: () => {
      if (done) return false;
      finish();
      return true;
    },
  };
}
