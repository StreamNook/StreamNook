// @vitest-environment node
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

// The suite runs in node: the module listens for `online` on `window` when it
// loads, so a window must exist before the static import below.
vi.hoisted(() => {
  (globalThis as unknown as { window: EventTarget }).window = new EventTarget();
});

import { failedAmong, isBadgeArtFailed, markBadgeArtFailed, subscribeBadgeArtFailures } from './badgeArtFailures';

describe('badge art failures', () => {
  beforeAll(() => {
    vi.useFakeTimers();
  });
  afterAll(() => {
    vi.useRealTimers();
  });
  beforeEach(() => {
    // Start every case from an empty tracker, with no expiry timer left pending.
    (globalThis as unknown as { window: EventTarget }).window.dispatchEvent(new Event('online'));
    vi.runOnlyPendingTimers();
  });

  it('marks a url failed and reports it only to rows that show it', () => {
    markBadgeArtFailed('u');
    expect(isBadgeArtFailed('u')).toBe(true);
    expect(failedAmong(['a', 'u'])).toBe('u\n');
    expect(failedAmong(['a'])).toBe('');
  });

  it('tells a listener once for a new url and not for a repeat', () => {
    const listener = vi.fn();
    const off = subscribeBadgeArtFailures(listener);
    markBadgeArtFailed('fresh');
    markBadgeArtFailed('fresh');
    expect(listener).toHaveBeenCalledTimes(1);
    off();
  });

  it('forgets a failure after the retry window and tells listeners', () => {
    markBadgeArtFailed('u');
    const listener = vi.fn();
    const off = subscribeBadgeArtFailures(listener);
    vi.advanceTimersByTime(61_000);
    expect(failedAmong(['u'])).toBe('');
    expect(listener).toHaveBeenCalled();
    off();
  });

  it('tells a listener again when the expiry timer fires', () => {
    markBadgeArtFailed('u2');
    const listener = vi.fn();
    const off = subscribeBadgeArtFailures(listener);
    listener.mockClear();
    vi.advanceTimersByTime(61_000);
    expect(listener).toHaveBeenCalledTimes(1);
    off();
  });

  it('coming back online clears a fresh mark', () => {
    markBadgeArtFailed('u3');
    expect(isBadgeArtFailed('u3')).toBe(true);
    (globalThis as unknown as { window: EventTarget }).window.dispatchEvent(new Event('online'));
    expect(isBadgeArtFailed('u3')).toBe(false);
  });
});
