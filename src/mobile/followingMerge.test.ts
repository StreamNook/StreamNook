import { describe, it, expect } from 'vitest';
import { followsKick, phoneFollowedLive } from './followingMerge';
import type { FollowingLists, TwitchStream } from '../types';
import type { ProviderId } from '../types/providers';

const tw = (login: string, viewers: number): TwitchStream =>
  ({ id: `t-${login}`, user_id: '1', user_login: login, user_name: login, viewer_count: viewers }) as TwitchStream;

const row = (provider: ProviderId, login: string, viewers: number): TwitchStream =>
  ({
    id: `${provider}-${login}`,
    user_id: '1',
    user_login: login,
    user_name: login,
    viewer_count: viewers,
    is_live: true,
    provider,
    key: `${provider}:${login}`,
    watch_url: '',
  }) as unknown as TwitchStream;

const lists = (followed_live: TwitchStream[], platforms: string[] = []): FollowingLists => ({
  favorites: [],
  live: [],
  offline: [],
  followed_live,
  platforms,
});

describe('phoneFollowedLive', () => {
  it("keeps Rust's order and both xqc rows in their own key spaces", () => {
    const out = phoneFollowedLive(lists([tw('xqc', 10), row('kick', 'xqc', 10), tw('small', 1)]));
    expect(out.map((s) => `${s.provider ?? 'twitch'}:${s.user_login}`)).toEqual([
      'twitch:xqc',
      'kick:xqc',
      'twitch:small',
    ]);
  });

  it('leaves out platforms the phone cannot watch', () => {
    const out = phoneFollowedLive(lists([tw('a', 1), row('youtube', 'b', 9), row('tiktok', 'c', 9)]));
    expect(out.map((s) => s.user_login)).toEqual(['a']);
  });

  it('is empty before Rust has built the list', () => {
    expect(phoneFollowedLive({ favorites: [], live: [], offline: [] })).toEqual([]);
  });
});

describe('followsKick', () => {
  it('reads the platforms Rust says you follow on', () => {
    expect(followsKick(lists([], ['twitch', 'kick']))).toBe(true);
    expect(followsKick(lists([], ['twitch']))).toBe(false);
    expect(followsKick({ favorites: [], live: [], offline: [] })).toBe(false);
  });
});
