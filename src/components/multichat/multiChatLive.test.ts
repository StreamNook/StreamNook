import { describe, expect, it } from 'vitest';
import { liveStats, liveWord, sourceKey, totalViewers, twitchStateLogin, type TwitchLiveState } from './multiChatLive';

const twitch = (entries: Record<string, TwitchLiveState>) => new Map(Object.entries(entries));

describe('liveStats', () => {
  it('reads Twitch rows from the channel-state map by bare lowercased login', () => {
    const [row] = liveStats(
      [{ channel: 'Kathi', channelName: 'kathi', provider: 'twitch', channelId: '1' }],
      twitch({ kathi: { viewer_count: 1200, viewers_at: 10 } }),
      {},
    );
    expect(row).toMatchObject({ key: 'twitch:kathi', isLive: true, known: true, count: 1200 });
  });

  it('treats an absent provider as Twitch', () => {
    const [row] = liveStats([{ channel: 'elly' }], twitch({ elly: { viewer_count: 5, viewers_at: 1 } }), {});
    expect(row).toMatchObject({ key: 'twitch:elly', provider: 'twitch', isLive: true });
  });

  it('reads an answered Twitch channel with no count as offline', () => {
    const [row] = liveStats([{ channel: 'suki' }], twitch({ suki: { viewer_count: null, viewers_at: 3 } }), {});
    expect(row).toMatchObject({ isLive: false, known: true });
    expect(liveWord(row)).toBe('Offline');
  });

  it('leaves a Twitch channel Rust has not answered for unknown', () => {
    const rows = liveStats(
      [{ channel: 'robits' }, { channel: 'varidetta' }],
      twitch({ robits: { viewer_count: null, viewers_at: null } }),
      {},
    );
    for (const row of rows) {
      expect(row).toMatchObject({ isLive: false, known: false });
      expect(liveWord(row)).toBeNull();
    }
  });

  it('never lets a same-named source on another platform borrow the Twitch state', () => {
    const rows = liveStats(
      [
        { channel: 'thepeanut', provider: 'twitch' },
        { channel: 'thepeanut', provider: 'kick' },
      ],
      twitch({ thepeanut: { viewer_count: 900, viewers_at: 1 } }),
      { 'kick:thepeanut': { is_live: false, viewer_count: null } },
    );
    expect(rows.map((r) => [r.key, r.isLive])).toEqual([
      ['twitch:thepeanut', true],
      ['kick:thepeanut', false],
    ]);
  });

  it('reads provider rows from adapter meta under the composite key', () => {
    const rows = liveStats(
      [
        { channel: 'IlloJuan', provider: 'kick' },
        { channel: '@Amane', provider: 'youtube' },
        { channel: 'miiikumo', provider: 'tiktok' },
      ],
      new Map(),
      {
        'kick:illojuan': { is_live: true, viewer_count: 40_000 },
        'youtube:@Amane': { is_live: true, viewer_count: 300 },
        'tiktok:miiikumo': null,
      },
    );
    expect(rows.map((r) => [r.key, r.isLive, r.known])).toEqual([
      ['kick:illojuan', true, true],
      ['youtube:@Amane', true, true],
      ['tiktok:miiikumo', false, false],
    ]);
  });

  it('keys YouTube rows case-sensitively, the same as the tab key', () => {
    const src = { channel: 'UCabcDEF', provider: 'youtube' as const };
    expect(sourceKey(src)).toBe('youtube:UCabcDEF');
    const [row] = liveStats([src], new Map(), { 'youtube:ucabcdef': { is_live: true } });
    expect(row.known).toBe(false);
  });

  it('drops a stale count from an offline provider and totals only live rows', () => {
    const rows = liveStats(
      [{ channel: 'a', provider: 'kick' }, { channel: 'b' }, { channel: 'c' }],
      twitch({ b: { viewer_count: 20, viewers_at: 1 }, c: { viewer_count: null, viewers_at: 1 } }),
      { 'kick:a': { is_live: false, viewer_count: 99 } },
    );
    expect(rows[0].count).toBeNull();
    expect(totalViewers(rows)).toBe(20);
  });

  it('trims and lowercases the login the store is read with', () => {
    expect(twitchStateLogin({ channel: ' Elajjaz ' })).toBe('elajjaz');
  });
});
