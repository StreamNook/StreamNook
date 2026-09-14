import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import {
  createLiveEdgeTracker,
  BEHIND_LIVE_ENTER_SECS,
  BEHIND_LIVE_EXIT_SECS,
} from './liveEdge';

/** Mirrors PEAK_WINDOW_MS in liveEdge.ts; the constant is internal on purpose,
 *  so the test states the wait it depends on rather than importing it. */
const PEAK_WINDOW_MS_TEST = 6000;

/** Minimal stand-in for the parts of HTMLVideoElement the tracker reads. */
function fakeVideo(bufferedEnd: number, currentTime: number) {
  return {
    currentTime,
    buffered: {
      length: 1,
      start: () => 0,
      end: () => bufferedEnd,
    },
  } as unknown as HTMLVideoElement;
}

/** Replay one HLS sawtooth: the buffered end jumps by `segment` seconds every
 *  `segment` seconds while currentTime advances smoothly, which is exactly what
 *  made the old raw measure swing. `steadyBehind` is the true distance. */
function sawtooth(segment: number, steadyBehind: number, seconds: number, stepMs = 250) {
  const out: { bufferedEnd: number; currentTime: number }[] = [];
  for (let t = 0; t < seconds; t += stepMs / 1000) {
    const currentTime = t;
    // The edge only moves in whole segments.
    const appended = Math.floor((t + steadyBehind) / segment) * segment;
    out.push({ bufferedEnd: appended, currentTime });
  }
  return out;
}

describe('live edge tracker', () => {
  beforeEach(() => { vi.useFakeTimers({ toFake: ['performance'] }); });
  afterEach(() => { vi.useRealTimers(); });

  it('reports a steady value across a sawtooth that swings by a whole segment', () => {
    const tracker = createLiveEdgeTracker();
    const readings: number[] = [];
    // Prime the window first, then record: the peak needs a full period in view.
    const frames = sawtooth(2, 6, 20);
    frames.forEach((f, i) => {
      const v = tracker.behind(fakeVideo(f.bufferedEnd, f.currentTime));
      vi.advanceTimersByTime(250);
      if (i > 24) readings.push(v);
    });
    const spread = Math.max(...readings) - Math.min(...readings);
    // The raw measure would swing by the full 2 s segment; the peak barely moves.
    expect(spread).toBeLessThan(0.6);
    // And it lands on the true distance rather than the trough.
    expect(Math.max(...readings)).toBeGreaterThanOrEqual(5.5);
  });

  it('never reports the trough, which is what made LIVE flicker', () => {
    const tracker = createLiveEdgeTracker();
    let min = Infinity;
    const frames = sawtooth(2, 6, 20);
    frames.forEach((f, i) => {
      const v = tracker.behind(fakeVideo(f.bufferedEnd, f.currentTime));
      vi.advanceTimersByTime(250);
      if (i > 24) min = Math.min(min, v);
    });
    // Raw troughs reach ~4 s on this stream; the tracker stays above them.
    expect(min).toBeGreaterThan(4.5);
  });

  it('holds LIVE through the whole normal latency band', () => {
    // A healthy stream at 4-8 s behind must never show a timestamp.
    for (const steady of [4, 6, 8]) {
      const tracker = createLiveEdgeTracker();
      let flipped = false;
      sawtooth(2, steady, 30).forEach((f) => {
        if (tracker.isBehind(fakeVideo(f.bufferedEnd, f.currentTime))) flipped = true;
        vi.advanceTimersByTime(250);
      });
      expect(flipped, `steady ${steady}s behind should still read LIVE`).toBe(false);
    }
  });

  it('applies hysteresis so a value sitting on the threshold cannot flap', () => {
    const tracker = createLiveEdgeTracker();
    const at = (behind: number) => fakeVideo(100 + behind, 100);
    // Each step waits out the peak window, so the reading is the new value and
    // the only thing under test is the latch.
    // Past the window, not exactly on it: a sample landing on the cutoff is
    // kept (the comparison is strict), which is fine in real use but would
    // make this assertion depend on a boundary rather than on the latch.
    const settle = (behind: number) => {
      vi.advanceTimersByTime(PEAK_WINDOW_MS_TEST + 250);
      return tracker.isBehind(at(behind));
    };
    expect(settle(BEHIND_LIVE_EXIT_SECS + 1)).toBe(true);
    // Between the two thresholds it must STAY behind, not bounce back.
    expect(settle(BEHIND_LIVE_ENTER_SECS + 2)).toBe(true);
    // Only below the lower threshold does it return to LIVE.
    expect(settle(BEHIND_LIVE_ENTER_SECS - 1)).toBe(false);
    // And it does not immediately flip back on a value between them.
    expect(settle(BEHIND_LIVE_EXIT_SECS - 1)).toBe(false);
  });

  it('drops the window on a seek, so Go Live reads LIVE at once', () => {
    const tracker = createLiveEdgeTracker();
    // Sitting a long way back.
    expect(tracker.isBehind(fakeVideo(1000, 940))).toBe(true);
    // Go Live: the playhead jumps to the edge. Same media timeline, so the
    // source-change rule does not apply; only the seek rule can catch this.
    vi.advanceTimersByTime(250);
    expect(tracker.behind(fakeVideo(1000, 996))).toBeLessThan(10);
    expect(tracker.isBehind(fakeVideo(1000, 996))).toBe(false);
  });

  it('starts over when the buffer jumps backwards (channel or quality change)', () => {
    const tracker = createLiveEdgeTracker();
    tracker.behind(fakeVideo(1000, 940)); // 60 s behind, a big peak
    vi.advanceTimersByTime(250);
    // New source: media time restarts near zero.
    const after = tracker.behind(fakeVideo(30, 24));
    expect(after).toBeLessThan(10);
  });

  it('reports zero with nothing buffered instead of a negative or NaN', () => {
    const tracker = createLiveEdgeTracker();
    const empty = { currentTime: 5, buffered: { length: 0, start: () => 0, end: () => 0 } } as unknown as HTMLVideoElement;
    expect(tracker.behind(empty)).toBe(0);
  });
});
