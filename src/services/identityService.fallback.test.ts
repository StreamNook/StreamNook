// isIdentityFallback tells a failed identity read's stand-in from a real
// "chose nothing" answer, on the first read and on cache hits.
import { describe, expect, it, vi } from 'vitest';

const failing = new Set<string>();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: async (cmd: string, args: { userId: string }) => {
    if (failing.has(args.userId)) throw new Error('offline');
    if (cmd === 'get_streamnook_identity') {
      return { twitch_user_id: args.userId, customized: false, badges: [], paint: null, updated_at: null };
    }
    return { twitch_user_id: args.userId, customized: false, badges: [], seventv: null, updated_at: null };
  },
}));

import { getIdentityWithCache, getResolvedIdentity, isIdentityFallback } from './identityService';

describe('isIdentityFallback', () => {
  it('marks the stand-in of a failed read, also when it is served again from the cache', async () => {
    failing.add('10');
    const loadout = await getIdentityWithCache('10');
    const resolved = await getResolvedIdentity('10');
    expect(isIdentityFallback(loadout)).toBe(true);
    expect(isIdentityFallback(resolved)).toBe(true);
    failing.delete('10');
    expect(isIdentityFallback(await getIdentityWithCache('10'))).toBe(true);
    expect(isIdentityFallback(await getResolvedIdentity('10'))).toBe(true);
  });

  it('a failed read keeps the normal 5-minute retry, so chat never asks a down server more often', async () => {
    vi.useFakeTimers({ toFake: ['Date'] });
    try {
      vi.setSystemTime(new Date('2026-10-31T12:00:00Z'));
      failing.add('12');
      expect(isIdentityFallback(await getResolvedIdentity('12'))).toBe(true);
      failing.delete('12');
      vi.setSystemTime(new Date('2026-10-31T12:04:00Z'));
      expect(isIdentityFallback(await getResolvedIdentity('12'))).toBe(true);
      vi.setSystemTime(new Date('2026-10-31T12:05:01Z'));
      expect(isIdentityFallback(await getResolvedIdentity('12'))).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it('a real answer that chose nothing is not a fallback', async () => {
    expect(isIdentityFallback(await getIdentityWithCache('11'))).toBe(false);
    expect(isIdentityFallback(await getResolvedIdentity('11'))).toBe(false);
    expect(isIdentityFallback(null)).toBe(false);
  });
});
