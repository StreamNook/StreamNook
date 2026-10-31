import { describe, it, expect } from 'vitest';
import { followingSections, followsKick } from './followingMerge';
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

const name = (s: TwitchStream) => `${s.provider ?? 'twitch'}:${s.user_login}`;

const lists = (favorites: TwitchStream[], live: TwitchStream[], platforms: string[] = []): FollowingLists => ({
  favorites,
  live,
  offline: [],
  followed_live: [...favorites, ...live],
  platforms,
});

describe('followingSections', () => {
  it("puts live favorites on the shelf and the rest under it, in Rust's order", () => {
    const s = followingSections(lists([tw('fav', 5)], [tw('xqc', 10), row('kick', 'xqc', 10), tw('small', 1)]), null);
    expect(s.favorites.map(name)).toEqual(['twitch:fav']);
    expect(s.rest.map(name)).toEqual(['twitch:xqc', 'kick:xqc', 'twitch:small']);
  });

  it('leaves out platforms the phone cannot watch, on the shelf too', () => {
    const s = followingSections(lists([row('youtube', 'yt', 9), tw('fav', 1)], [tw('a', 1), row('tiktok', 'c', 9)]), null);
    expect(s.favorites.map(name)).toEqual(['twitch:fav']);
    expect(s.rest.map(name)).toEqual(['twitch:a']);
  });

  it('keeps one platform under the filter, on both parts', () => {
    const s = followingSections(lists([tw('fav', 5), row('kick', 'kfav', 2)], [tw('a', 3), row('kick', 'k', 4)]), 'kick');
    expect(s.favorites.map(name)).toEqual(['kick:kfav']);
    expect(s.rest.map(name)).toEqual(['kick:k']);
    expect(s.mixed).toBe(false);
  });

  it('counts the shelf and the list together for platform marks', () => {
    expect(followingSections(lists([row('kick', 'kfav', 2)], [tw('a', 3)]), null).mixed).toBe(true);
    expect(followingSections(lists([tw('fav', 2)], [tw('a', 3)]), null).mixed).toBe(false);
    expect(followingSections(lists([], [tw('a', 3), row('kick', 'k', 4)]), null).mixed).toBe(true);
  });

  it('is empty before Rust has built the lists', () => {
    const s = followingSections({ favorites: [], live: [], offline: [] }, null);
    expect(s).toEqual({ favorites: [], rest: [], mixed: false });
  });
});

describe('followsKick', () => {
  it('reads the platforms Rust says you follow on', () => {
    expect(followsKick(lists([], [], ['twitch', 'kick']))).toBe(true);
    expect(followsKick(lists([], [], ['twitch']))).toBe(false);
    expect(followsKick({ favorites: [], live: [], offline: [] })).toBe(false);
  });
});
