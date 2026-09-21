import { describe, expect, it } from 'vitest';
import { REFRESH_AHEAD_MS, RETRY_WINDOW_MS, shouldAttemptSevenTvRefresh } from './sevenTvSession';

const DAY = 24 * 60 * 60 * 1000;

describe('shouldAttemptSevenTvRefresh', () => {
  const now = Date.parse('2026-09-20T12:00:00Z');
  const sec = (ms: number) => Math.floor(ms / 1000);

  it('never refreshes when nothing is stored', () => {
    expect(shouldAttemptSevenTvRefresh({ expiresAtSec: null, nowMs: now, lastAttemptMs: null })).toBe(false);
  });

  it('leaves a fresh token alone', () => {
    expect(
      shouldAttemptSevenTvRefresh({ expiresAtSec: sec(now + 20 * DAY), nowMs: now, lastAttemptMs: null }),
    ).toBe(false);
  });

  it('refreshes inside the lead window', () => {
    expect(
      shouldAttemptSevenTvRefresh({
        expiresAtSec: sec(now + REFRESH_AHEAD_MS - DAY),
        nowMs: now,
        lastAttemptMs: null,
      }),
    ).toBe(true);
  });

  it('refreshes an already expired token', () => {
    expect(
      shouldAttemptSevenTvRefresh({ expiresAtSec: sec(now - 3 * DAY), nowMs: now, lastAttemptMs: null }),
    ).toBe(true);
  });

  it('does not retry inside the retry window', () => {
    expect(
      shouldAttemptSevenTvRefresh({
        expiresAtSec: sec(now - DAY),
        nowMs: now,
        lastAttemptMs: now - RETRY_WINDOW_MS + 60_000,
      }),
    ).toBe(false);
  });

  it('retries once the retry window has passed', () => {
    expect(
      shouldAttemptSevenTvRefresh({
        expiresAtSec: sec(now - DAY),
        nowMs: now,
        lastAttemptMs: now - RETRY_WINDOW_MS - 60_000,
      }),
    ).toBe(true);
  });
});
