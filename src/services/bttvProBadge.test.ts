// resolveBttvProUrl: a Pro badge, a real "no Pro", and a lookup that could not
// be asked are three different answers.
import { describe, expect, it, vi } from 'vitest';

const reply = { kind: 'pro' as 'pro' | 'none' | 'fail' };
vi.mock('@tauri-apps/api/core', () => ({
  invoke: async () => {
    if (reply.kind === 'fail') throw new Error('BTTV socket: could not connect');
    return reply.kind === 'pro' ? { url: 'https://cdn.betterttv.net/badges/pro/x.webp', started_at: null, glow: false } : null;
  },
}));

import { resolveBttvProUrl } from './bttvProBadge';

describe('resolveBttvProUrl', () => {
  it('answers the badge, null for no Pro, and undefined when BTTV could not be asked', async () => {
    reply.kind = 'pro';
    expect(await resolveBttvProUrl('5')).toBe('https://cdn.betterttv.net/badges/pro/x.webp');
    reply.kind = 'none';
    expect(await resolveBttvProUrl('5')).toBeNull();
    reply.kind = 'fail';
    expect(await resolveBttvProUrl('5')).toBeUndefined();
  });
});
