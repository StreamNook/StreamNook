// How far behind the live edge the playhead is, stated so it does not flicker.
//
// The naive measure, `buffered.end(last) - currentTime`, is a SAWTOOTH. HLS
// appends whole segments, so the buffered end jumps forward by one segment
// (~2 s here, up to targetDuration) and then sits still while `currentTime`
// keeps advancing. The raw number therefore falls for a segment's worth of
// time, jumps back up, and repeats, swinging by the segment duration forever
// on a perfectly healthy stream.
//
// Two things were built on that raw number and both shook:
//
//   * the player's time display flipped between "LIVE" and a timestamp,
//   * the broadcast timeline's playhead slid back and forth by seconds.
//
// The fix is to report the sawtooth's PEAK over a short window. The peak is
// also the honest value: a segment's content was produced roughly one segment
// duration before it appended, so the gap is widest, and truest, right after
// an append. Taking the peak from real buffered samples keeps this grounded in
// what has actually been downloaded. It deliberately does NOT extrapolate the
// edge with wall-clock time: `hls.latency` already does that (it adds playlist
// `age`) and a stale playlist inflates it by a second per second while playback
// is fine, which is a reliable source of false latency diagnoses.

/** Peak window. Must exceed the largest expected segment duration so a whole
 *  sawtooth period is always in view; 6 s covers Twitch's 2 s segments and the
 *  6 s targetDuration seen before the relay's rewrite. */
const PEAK_WINDOW_MS = 6000;

/** A buffered end that moves back further than this is a new source (channel
 *  change, quality switch, rewind), not a sawtooth: start the window over. */
const SOURCE_CHANGE_DROP_SECS = 10;

/** A fall in the measured distance larger than any plausible segment is a SEEK,
 *  not a sawtooth trough: the playhead jumped forward (Go Live, or a scrub into
 *  the edge). Without this the peak window would keep reporting the old, larger
 *  distance for its full length, so pressing Go Live would leave the label
 *  reading "behind" for seconds afterwards. Sized above the 6 s targetDuration
 *  ceiling so a real trough can never trip it. */
const SEEK_DROP_SECS = 7;

/** Callers sample at ~4 Hz, so the window never holds many entries. Bounded
 *  anyway, because a caller that samples per frame must not grow this. */
const MAX_SAMPLES = 64;

/**
 * Show a timestamp instead of LIVE only past this. It sits deliberately ABOVE
 * normal operating latency: a healthy StreamNook stream rides 4-8 s behind
 * (`liveSyncDuration` 8, or 4 on the low-latency path, over a targetDuration
 * floor), so the old 5 s threshold sat INSIDE the normal band and a healthy
 * stream crossed it twice per segment. Anyone who has actually scrubbed back
 * is far past 15 s.
 */
export const BEHIND_LIVE_EXIT_SECS = 15;

/** ...and return to LIVE below this. The gap between the two is the
 *  hysteresis: with one threshold, any noise at all flips the label. */
export const BEHIND_LIVE_ENTER_SECS = 10;

export interface LiveEdgeTracker {
  /** Smoothed seconds behind the live edge. Safe to call as often as you like. */
  behind(video: HTMLVideoElement): number;
  /** True when the playhead should read as behind rather than LIVE. Applies
   *  hysteresis, so it needs to be called on the same tracker each time. */
  isBehind(video: HTMLVideoElement): boolean;
  /** The playhead jumped by `playheadDeltaSecs` (a seek; positive = forward).
   *  Re-base the window so it describes where playback now is, instead of
   *  holding the pre-seek distance for a full window: a small forward seek
   *  falls under SEEK_DROP_SECS, so the peak alone would keep reporting the
   *  old value for up to 6 s. */
  shift(playheadDeltaSecs: number): void;
  /** Forget history: a new stream, a rewind, or a source swap. */
  reset(): void;
}

export function createLiveEdgeTracker(): LiveEdgeTracker {
  let samples: { at: number; behind: number }[] = [];
  let lastEnd = 0;
  let behindState = false;

  const measure = (video: HTMLVideoElement): number => {
    const b = video.buffered;
    if (b.length === 0) return 0;
    const end = b.end(b.length - 1);
    if (end < lastEnd - SOURCE_CHANGE_DROP_SECS) samples = [];
    lastEnd = end;

    const raw = Math.max(0, end - video.currentTime);
    const now = performance.now();

    // A jump forward in the playhead is a seek, so the window's history no
    // longer describes where we are. Drop it before it can hold the old value.
    let prevPeak = 0;
    for (const s of samples) if (s.behind > prevPeak) prevPeak = s.behind;
    if (raw < prevPeak - SEEK_DROP_SECS) samples = [];

    samples.push({ at: now, behind: raw });
    const cutoff = now - PEAK_WINDOW_MS;
    while (samples.length > 0 && samples[0].at < cutoff) samples.shift();
    while (samples.length > MAX_SAMPLES) samples.shift();

    let peak = raw;
    for (const s of samples) if (s.behind > peak) peak = s.behind;
    return peak;
  };

  return {
    behind: measure,
    isBehind(video) {
      const behind = measure(video);
      // Latch: only cross a threshold, never sit on one.
      if (behindState) {
        if (behind <= BEHIND_LIVE_ENTER_SECS) behindState = false;
      } else if (behind >= BEHIND_LIVE_EXIT_SECS) {
        behindState = true;
      }
      return behindState;
    },
    shift(playheadDeltaSecs) {
      if (!Number.isFinite(playheadDeltaSecs) || playheadDeltaSecs === 0) return;
      for (const s of samples) s.behind = Math.max(0, s.behind - playheadDeltaSecs);
    },
    reset() {
      samples = [];
      lastEnd = 0;
      behindState = false;
    },
  };
}
