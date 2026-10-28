import { describe, expect, it } from 'vitest';
import { channelOf, heardAgo, heardClock, heardDay, heardInto, mediaMark } from './songWords';

const NOW = new Date(2026, 9, 27, 12, 0, 0).getTime();

describe('Your songs words', () => {
  it('says how long ago in the shortest true words', () => {
    expect(heardAgo(NOW - 20_000, NOW)).toBe('just now');
    expect(heardAgo(NOW + 5_000, NOW)).toBe('just now');
    expect(heardAgo(NOW - 12 * 60_000, NOW)).toBe('12m ago');
    expect(heardAgo(NOW - 5 * 3_600_000, NOW)).toBe('5h ago');
    expect(heardAgo(NOW - 3 * 86_400_000, NOW)).toBe('3d ago');
    expect(heardAgo(NOW - 30 * 86_400_000, NOW)).toBe('Sep 27');
    expect(heardAgo(new Date(2025, 0, 3).getTime(), NOW)).toBe('Jan 3, 2025');
  });

  const live = { provider: 'twitch' as const, channel_login: 'xqc', channel_name: 'xQc', media: 'live' as const };

  it('names the channel, and marks what was not live', () => {
    expect(channelOf(live)).toBe('xQc');
    expect(channelOf({ ...live, channel_name: null })).toBe('xqc');
    expect(mediaMark(live)).toBe(null);
    expect(mediaMark({ ...live, media: 'video' })).toBe('VOD');
    expect(mediaMark({ ...live, media: 'clip' })).toBe('Clip');
  });

  it('says how far into the broadcast or video it was heard', () => {
    expect(heardInto(live)).toBe(null);
    expect(heardInto({ ...live, position_secs: 30 })).toBe('at the start');
    expect(heardInto({ ...live, position_secs: 14 * 60 + 59 })).toBe('14m in');
    expect(heardInto({ ...live, position_secs: 2 * 3600 + 14 * 60 })).toBe('2h 14m in');
    expect(heardInto({ ...live, position_secs: 3 * 3600 + 20 })).toBe('3h in');
    expect(heardInto({ ...live, media: 'video', position_secs: 3600 + 23 * 60 + 45 })).toBe('at 1:23:45');
    expect(heardInto({ ...live, media: 'video', position_secs: 245 })).toBe('at 4:05');
    expect(heardInto({ ...live, media: 'clip', position_secs: 12 })).toBe(null);
    expect(heardInto({ ...live, position_secs: -1 })).toBe(null);
  });

  it('names the day and the time of day', () => {
    expect(heardDay(new Date(2026, 9, 27, 0, 5).getTime(), NOW)).toBe('Today');
    expect(heardDay(new Date(2026, 9, 26, 23, 59).getTime(), NOW)).toBe('Yesterday');
    expect(heardDay(new Date(2026, 9, 21, 9).getTime(), NOW)).toBe('Oct 21');
    expect(heardDay(new Date(2025, 11, 31, 9).getTime(), NOW)).toBe('Dec 31, 2025');
    expect(heardClock(new Date(2026, 9, 27, 21, 14).getTime())).toBe('9:14 PM');
  });
});
