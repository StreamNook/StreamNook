import { describe, expect, it } from 'vitest';
import { liveStats, liveWord, sourceKey, totalViewers } from './tabLive';

describe('liveStats', () => {
  it('reads each source from the Rust rows by its tab key', () => {
    const [row] = liveStats(
      [{ channel: 'Kathi', channelName: 'kathi', provider: 'twitch', channelId: '1' }],
      { 'twitch:kathi': { live: true, viewer_count: 1200 } },
    );
    expect(row).toMatchObject({ key: 'twitch:kathi', isLive: true, known: true, count: 1200 });
    expect(liveWord(row)).toBe('Live');
  });

  it('treats an absent provider as Twitch', () => {
    const [row] = liveStats([{ channel: 'elly' }], { 'twitch:elly': { live: true, viewer_count: 5 } });
    expect(row).toMatchObject({ key: 'twitch:elly', provider: 'twitch', isLive: true });
  });

  it('reads an answered channel that is not live as offline', () => {
    const [row] = liveStats([{ channel: 'suki' }], { 'twitch:suki': { live: false, viewer_count: null } });
    expect(row).toMatchObject({ isLive: false, known: true });
    expect(liveWord(row)).toBe('Offline');
  });

  it('leaves a source Rust has not answered for unknown', () => {
    const [row] = liveStats([{ channel: 'robits' }], {});
    expect(row).toMatchObject({ isLive: false, known: false });
    expect(liveWord(row)).toBeNull();
  });

  it('never lets a same-named source on another platform borrow a row', () => {
    const rows = liveStats(
      [
        { channel: 'thepeanut', provider: 'twitch' },
        { channel: 'thepeanut', provider: 'kick' },
      ],
      { 'twitch:thepeanut': { live: true, viewer_count: 900 } },
    );
    expect(rows.map((r) => [r.key, r.isLive, r.known])).toEqual([
      ['twitch:thepeanut', true, true],
      ['kick:thepeanut', false, false],
    ]);
  });

  it('keys YouTube rows case-sensitively, the same as the tab key', () => {
    const src = { channel: 'UCabcDEF', provider: 'youtube' as const };
    expect(sourceKey(src)).toBe('youtube:UCabcDEF');
    expect(liveStats([src], { 'youtube:ucabcdef': { live: true, viewer_count: 1 } })[0].known).toBe(false);
  });

  it('drops a stale count from an offline source and totals only live rows', () => {
    const rows = liveStats([{ channel: 'a', provider: 'kick' }, { channel: 'b' }, { channel: 'c' }], {
      'kick:a': { live: false, viewer_count: 99 },
      'twitch:b': { live: true, viewer_count: 20 },
      'twitch:c': { live: false, viewer_count: null },
    });
    expect(rows[0].count).toBeNull();
    expect(totalViewers(rows)).toBe(20);
  });
});
