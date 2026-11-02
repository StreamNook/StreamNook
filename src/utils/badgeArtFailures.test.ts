// @vitest-environment node
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';

// The suite runs in node: the module listens for `online` on `window` when it
// loads, so a window must exist before the static import below. A fake Image
// stands in for the off-screen probe: urls in `loadable` load, others fail.
const loadable = vi.hoisted(() => {
  (globalThis as unknown as { window: EventTarget }).window = new EventTarget();
  const ok = new Set<string>();
  class FakeImage {
    onload: (() => void) | null = null;
    onerror: (() => void) | null = null;
    set src(url: string) {
      queueMicrotask(() => (ok.has(url) ? this.onload?.() : this.onerror?.()));
    }
  }
  (globalThis as unknown as { Image: unknown }).Image = FakeImage;
  return ok;
});

import { failedAmong, isBadgeArtFailed, markBadgeArtFailed, subscribeBadgeArtFailures } from './badgeArtFailures';

const flush = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

describe('badge art failures', () => {
  beforeAll(() => {
    vi.useFakeTimers();
  });
  afterAll(() => {
    vi.useRealTimers();
  });

  it('marks a url failed and reports it only to rows that show it', () => {
    markBadgeArtFailed('u');
    expect(isBadgeArtFailed('u')).toBe(true);
    expect(failedAmong(['a', 'u'])).toBe('u\n');
    expect(failedAmong(['a'])).toBe('');
  });

  it('tells a listener once for a new url and not for a repeat', async () => {
    const listener = vi.fn();
    const off = subscribeBadgeArtFailures(listener);
    markBadgeArtFailed('fresh');
    markBadgeArtFailed('fresh');
    await flush();
    expect(listener).toHaveBeenCalledTimes(1);
    off();
  });

  it('tells rows once for a burst of failures', async () => {
    const listener = vi.fn();
    const off = subscribeBadgeArtFailures(listener);
    for (const u of ['b1', 'b2', 'b3', 'b4']) markBadgeArtFailed(u);
    await flush();
    expect(listener).toHaveBeenCalledTimes(1);
    off();
  });

  it('keeps dead art failed after the retry: it never pops back into rows', async () => {
    markBadgeArtFailed('dead');
    const listener = vi.fn();
    const off = subscribeBadgeArtFailures(listener);
    await flush();
    listener.mockClear();
    vi.advanceTimersByTime(61_000);
    await flush();
    expect(isBadgeArtFailed('dead')).toBe(true);
    expect(listener).not.toHaveBeenCalled();
    off();
  });

  it('clears art once an off-screen retry loads it, and tells rows', async () => {
    markBadgeArtFailed('back');
    const listener = vi.fn();
    const off = subscribeBadgeArtFailures(listener);
    await flush();
    listener.mockClear();
    loadable.add('back');
    vi.advanceTimersByTime(61_000);
    await flush();
    expect(isBadgeArtFailed('back')).toBe(false);
    expect(listener).toHaveBeenCalledTimes(1);
    off();
  });

  it("clears a disk copy when Twitch's own copy loads", async () => {
    markBadgeArtFailed('asset://local/badge.png', 'https://static-cdn.jtvnw.net/badges/v1/x/3');
    loadable.add('https://static-cdn.jtvnw.net/badges/v1/x/3');
    vi.advanceTimersByTime(61_000);
    await flush();
    expect(isBadgeArtFailed('asset://local/badge.png')).toBe(false);
  });

  it('retries at once when the network comes back', async () => {
    markBadgeArtFailed('net');
    loadable.add('net');
    (globalThis as unknown as { window: EventTarget }).window.dispatchEvent(new Event('online'));
    await flush();
    expect(isBadgeArtFailed('net')).toBe(false);
  });
});
