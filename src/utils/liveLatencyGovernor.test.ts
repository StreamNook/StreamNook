import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import type Hls from 'hls.js';
import { startLatencyGovernor } from './liveLatencyGovernor';

/** Stand-in for the media element: a mutable playhead-to-edge distance and a
 *  generous forward buffer, so only the latency logic is under test. */
function fakeVideo(forward = 3.5) {
  return {
    paused: false,
    seeking: false,
    playbackRate: 1.0,
    currentTime: 100,
    buffered: { length: 1, start: () => 90, end: () => 100 + forward },
  } as unknown as HTMLVideoElement;
}
const fakeHls = { config: { liveSyncDuration: 3 } } as unknown as Hls;

describe('latency governor, latency-targeting hysteresis', () => {
  beforeEach(() => {
    // The governor schedules through `window`; the test runs in node.
    vi.stubGlobal('window', globalThis);
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('engages past the band, runs to the target, and lets go there', () => {
    const video = fakeVideo();
    let latency = 4.7; // a cold start, 1.7 s past a 3 s target
    const stop = startLatencyGovernor(fakeHls, video, {
      latencyTarget: () => 3,
      getLatency: () => latency,
      band: 0.75,
      gain: 0.12,
      ceiling: 1.05,
      floor: 0.8,
      engageSpan: 1.5,
      tickMs: 100,
    });
    vi.advanceTimersByTime(100);
    expect(video.playbackRate).toBeGreaterThan(1.0);
    // Inside the band but still above the target: the old behaviour released
    // here and parked the session at target + band for good. The finish runs
    // at the fixed finish rate, not a glide.
    latency = 3.5;
    vi.advanceTimersByTime(100);
    expect(video.playbackRate).toBeGreaterThan(1.0);
    for (let i = 0; i < 10; i++) vi.advanceTimersByTime(100);
    expect(video.playbackRate).toBe(1.03);
    // Within `release` of the target: done.
    latency = 3.1;
    vi.advanceTimersByTime(100);
    expect(video.playbackRate).toBe(1.0);
    stop();
  });

  it('does not re-engage on jitter inside the band', () => {
    const video = fakeVideo();
    let latency = 3.0;
    const stop = startLatencyGovernor(fakeHls, video, {
      latencyTarget: () => 3,
      getLatency: () => latency,
      band: 0.75,
      gain: 0.12,
      ceiling: 1.05,
      tickMs: 100,
    });
    for (const l of [3.4, 3.6, 2.7, 3.2, 3.7]) {
      latency = l;
      vi.advanceTimersByTime(100);
      expect(video.playbackRate).toBe(1.0);
    }
    stop();
  });

  it('eases back only past the band and lets go inside it', () => {
    const video = fakeVideo();
    let latency = 1.8; // 1.2 s ahead of a 3 s target
    const stop = startLatencyGovernor(fakeHls, video, {
      latencyTarget: () => 3,
      getLatency: () => latency,
      band: 0.75,
      gain: 0.12,
      slowRate: 0.97,
      tickMs: 100,
    });
    vi.advanceTimersByTime(100);
    expect(video.playbackRate).toBeLessThan(1.0);
    latency = 2.6; // inside the band: a slowed stream is what viewers hear as distortion
    vi.advanceTimersByTime(100);
    expect(video.playbackRate).toBe(1.0);
    stop();
  });

  it('snaps a residual rate back to exactly 1.0 so the pitch corrector disengages', () => {
    const video = fakeVideo();
    video.playbackRate = 1.004;
    const stop = startLatencyGovernor(fakeHls, video, {
      latencyTarget: () => 3,
      getLatency: () => 3.0,
      band: 0.75,
      tickMs: 100,
    });
    vi.advanceTimersByTime(100);
    expect(video.playbackRate).toBe(1.0);
    stop();
  });

  it('low-buffer protection still wins over a catch-up', () => {
    const video = fakeVideo(0.5); // under the 0.8 s floor
    const stop = startLatencyGovernor(fakeHls, video, {
      latencyTarget: () => 3,
      getLatency: () => 6,
      band: 0.75,
      gain: 0.12,
      ceiling: 1.05,
      floor: 0.8,
      slowRate: 0.97,
      tickMs: 100,
    });
    vi.advanceTimersByTime(100);
    expect(video.playbackRate).toBe(0.97);
    stop();
  });
});
