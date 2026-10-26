import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock('../utils/logger', () => ({ Logger: { warn: vi.fn(), debug: vi.fn(), info: vi.fn(), error: vi.fn() } }));

import { queueEmojiForCaching } from './emojiService';

const CAP = 1000;

beforeEach(() => {
  vi.useFakeTimers();
  invoke.mockReset();
  invoke.mockRejectedValue(new Error('404'));
});

afterEach(() => {
  vi.useRealTimers();
});

const flush = async () => {
  await vi.advanceTimersByTimeAsync(60);
  await vi.advanceTimersByTimeAsync(0);
};

const requested = () => invoke.mock.calls.filter(([cmd]) => cmd === 'get_emoji_image').map(([, a]) => a.codepoint);

it('the failed-emoji set holds 1000 codepoints; the oldest is asked for again', async () => {
  for (let i = 0; i <= CAP; i++) queueEmojiForCaching(`cp${i}`);
  await flush();
  expect(requested()).toHaveLength(CAP + 1);

  invoke.mockClear();
  // Still remembered as failed: never re-queued.
  queueEmojiForCaching('cp1');
  queueEmojiForCaching(`cp${CAP}`);
  // Evicted: queued like an emoji never tried.
  queueEmojiForCaching('cp0');
  await flush();
  expect(requested()).toEqual(['cp0']);
});
