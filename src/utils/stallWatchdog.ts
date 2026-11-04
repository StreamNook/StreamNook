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
