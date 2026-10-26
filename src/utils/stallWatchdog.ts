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
