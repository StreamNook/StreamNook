import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  RETRY_DELAYS_SECONDS,
  candidateUrls,
  createEmoteImageRetry,
  type EmoteImageProbe,
} from './emoteImageRetry';

/** A probe that records every URL and answers from a mutable set of live files. */
function fakeProbe() {
  const live = new Set<string>();
  const calls: string[] = [];
  let inFlight = 0;
  let maxInFlight = 0;
  const probe: EmoteImageProbe = async (url) => {
    calls.push(url);
    inFlight++;
    maxInFlight = Math.max(maxInFlight, inFlight);
    await Promise.resolve();
    inFlight--;
    return live.has(url);
  };
  return { probe, live, calls, maxInFlight: () => maxInFlight };
}

const SWEEP = candidateUrls('E1').length;

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('candidateUrls', () => {
  it('tries the preferred tier first, avif before webp, then the rest', () => {
    const urls = candidateUrls('E1', '2x');
    expect(urls.slice(0, 3)).toEqual([
      'https://cdn.7tv.app/emote/E1/2x.avif',
      'https://cdn.7tv.app/emote/E1/2x.webp',
      'https://cdn.7tv.app/emote/E1/1x.avif',
    ]);
    expect(urls).toHaveLength(8);
  });

  it('adds the legacy format only when the animation is known', () => {
    expect(candidateUrls('E1', '1x', true)).toContain('https://cdn.7tv.app/emote/E1/4x.gif');
    expect(candidateUrls('E1', '1x', false)).toContain('https://cdn.7tv.app/emote/E1/4x.png');
    expect(candidateUrls('E1')).toHaveLength(8);
  });
});

describe('emoteImageRetry', () => {
  it('sweeps once, then retries on the schedule with one probe at a time for many rows', async () => {
    const { probe, calls, maxInFlight } = fakeProbe();
    const retry = createEmoteImageRetry(probe);
    const rows = Array.from({ length: 20 }, () => vi.fn());
    for (const onResolved of rows) retry.subscribe('E1', { onResolved });

    await vi.advanceTimersByTimeAsync(0);
    expect(calls).toHaveLength(SWEEP);

    // Nothing runs before the first delay, then one sweep per delay.
    await vi.advanceTimersByTimeAsync(RETRY_DELAYS_SECONDS[0] * 1000 - 1);
    expect(calls).toHaveLength(SWEEP);
    await vi.advanceTimersByTimeAsync(1);
    expect(calls).toHaveLength(SWEEP * 2);

    await vi.advanceTimersByTimeAsync(RETRY_DELAYS_SECONDS[1] * 1000);
    expect(calls).toHaveLength(SWEEP * 3);

    expect(maxInFlight()).toBe(1);
    for (const onResolved of rows) expect(onResolved).not.toHaveBeenCalled();
  });

  it('tells every row the working URL once a file appears', async () => {
    const { probe, live } = fakeProbe();
    const retry = createEmoteImageRetry(probe);
    const rows = Array.from({ length: 20 }, () => vi.fn());
    for (const onResolved of rows) retry.subscribe('E1', { onResolved });
    await vi.advanceTimersByTimeAsync(0);

    live.add('https://cdn.7tv.app/emote/E1/2x.webp');
    await vi.advanceTimersByTimeAsync(RETRY_DELAYS_SECONDS[0] * 1000);

    for (const onResolved of rows) {
      expect(onResolved).toHaveBeenCalledTimes(1);
      expect(onResolved).toHaveBeenCalledWith('https://cdn.7tv.app/emote/E1/2x.webp');
    }
    expect(retry.subscriberCount('E1')).toBe(0);
  });

  it('gives a row mounted after success the working URL at once', async () => {
    const { probe, live, calls } = fakeProbe();
    live.add('https://cdn.7tv.app/emote/E1/1x.webp');
    const retry = createEmoteImageRetry(probe);
    retry.subscribe('E1', { onResolved: vi.fn() });
    await vi.advanceTimersByTimeAsync(0);

    const late = vi.fn();
    retry.subscribe('E1', { onResolved: late });
    expect(late).toHaveBeenCalledWith('https://cdn.7tv.app/emote/E1/1x.webp');
    await vi.advanceTimersByTimeAsync(600_000);
    expect(calls).toHaveLength(2);
  });

  it('stops retrying when every row unsubscribes', async () => {
    const { probe, calls } = fakeProbe();
    const retry = createEmoteImageRetry(probe);
    const leaves = Array.from({ length: 5 }, () => retry.subscribe('E1', { onResolved: vi.fn() }));
    await vi.advanceTimersByTimeAsync(0);
    expect(calls).toHaveLength(SWEEP);

    for (const leave of leaves) leave();
    expect(retry.subscriberCount('E1')).toBe(0);
    await vi.advanceTimersByTimeAsync(600_000);
    expect(calls).toHaveLength(SWEEP);
  });

  it('keeps retrying while one row remains', async () => {
    const { probe, calls } = fakeProbe();
    const retry = createEmoteImageRetry(probe);
    const leaves = [1, 2, 3].map(() => retry.subscribe('E1', { onResolved: vi.fn() }));
    await vi.advanceTimersByTimeAsync(0);
    leaves[0]();
    leaves[1]();
    await vi.advanceTimersByTimeAsync(RETRY_DELAYS_SECONDS[0] * 1000);
    expect(calls).toHaveLength(SWEEP * 2);
  });

  it('gives up after the last delay', async () => {
    const { probe, calls } = fakeProbe();
    const retry = createEmoteImageRetry(probe);
    retry.subscribe('E1', { onResolved: vi.fn() });
    await vi.advanceTimersByTimeAsync(600_000);
    expect(calls).toHaveLength(SWEEP * (RETRY_DELAYS_SECONDS.length + 1));

    await vi.advanceTimersByTimeAsync(3_600_000);
    expect(calls).toHaveLength(SWEEP * (RETRY_DELAYS_SECONDS.length + 1));
  });

  it('lets a row mounted long after giving up start over, but not right away', async () => {
    const { probe, calls } = fakeProbe();
    const retry = createEmoteImageRetry(probe);
    const first = retry.subscribe('E1', { onResolved: vi.fn() });
    await vi.advanceTimersByTimeAsync(600_000);
    const total = calls.length;
    first();

    retry.subscribe('E1', { onResolved: vi.fn() });
    await vi.advanceTimersByTimeAsync(0);
    expect(calls).toHaveLength(total);

    await vi.advanceTimersByTimeAsync(300_000);
    retry.subscribe('E1', { onResolved: vi.fn() });
    await vi.advanceTimersByTimeAsync(0);
    expect(calls.length).toBeGreaterThan(total);
  });

  it('keeps emote ids independent', async () => {
    const { probe, live, calls } = fakeProbe();
    const retry = createEmoteImageRetry(probe);
    const a = vi.fn();
    const b = vi.fn();
    retry.subscribe('E1', { onResolved: a });
    const leaveB = retry.subscribe('E2', { onResolved: b });
    await vi.advanceTimersByTimeAsync(0);
    expect(calls.filter((u) => u.includes('/E1/'))).toHaveLength(SWEEP);
    expect(calls.filter((u) => u.includes('/E2/'))).toHaveLength(SWEEP);

    live.add('https://cdn.7tv.app/emote/E2/1x.avif');
    await vi.advanceTimersByTimeAsync(RETRY_DELAYS_SECONDS[0] * 1000);
    expect(b).toHaveBeenCalledWith('https://cdn.7tv.app/emote/E2/1x.avif');
    expect(a).not.toHaveBeenCalled();
    expect(retry.subscriberCount('E1')).toBe(1);
    leaveB();
  });

  it('survives a row leaving and returning during a sweep without a second probe', async () => {
    const { probe, maxInFlight } = fakeProbe();
    const retry = createEmoteImageRetry(probe);
    const leave = retry.subscribe('E1', { onResolved: vi.fn() });
    leave();
    retry.subscribe('E1', { onResolved: vi.fn() });
    await vi.advanceTimersByTimeAsync(0);
    expect(maxInFlight()).toBe(1);
  });
});
