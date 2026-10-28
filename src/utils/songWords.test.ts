import { describe, expect, it } from 'vitest';
import { heardAgo, placeOf } from './songWords';

const NOW = Date.UTC(2026, 9, 27, 12, 0, 0);

describe('Your songs words', () => {
  it('says how long ago in the shortest true words', () => {
    expect(heardAgo(NOW - 20_000, NOW)).toBe('just now');
    expect(heardAgo(NOW + 5_000, NOW)).toBe('just now');
    expect(heardAgo(NOW - 12 * 60_000, NOW)).toBe('12m ago');
    expect(heardAgo(NOW - 5 * 3_600_000, NOW)).toBe('5h ago');
    expect(heardAgo(NOW - 3 * 86_400_000, NOW)).toBe('3d ago');
    expect(heardAgo(NOW - 30 * 86_400_000, NOW)).not.toMatch(/ago$/);
  });

  it('names the channel, and says when it was not live', () => {
    const live = { provider: 'twitch' as const, channel_login: 'xqc', channel_name: 'xQc', media: 'live' as const };
    expect(placeOf(live)).toBe('xQc');
    expect(placeOf({ ...live, channel_name: null })).toBe('xqc');
    expect(placeOf({ ...live, media: 'video' })).toBe('xQc · VOD');
    expect(placeOf({ ...live, media: 'clip' })).toBe('xQc · clip');
  });
});
