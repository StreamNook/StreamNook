import { describe, it, expect } from 'vitest';
import { mergeFollowedLive } from './followingMerge';
import type { TwitchStream } from '../types';
import type { ProviderStreamRow } from '../stores/followsStore';
import type { ProviderId } from '../types/providers';

const tw = (login: string, viewers: number): TwitchStream =>
  ({ id: `t-${login}`, user_id: '1', user_login: login, user_name: login, viewer_count: viewers }) as TwitchStream;

const row = (provider: ProviderId, login: string, viewers: number, isLive = true): ProviderStreamRow =>
  ({
    id: `${provider}-${login}`,
    user_id: '1',
    user_login: login,
    user_name: login,
    viewer_count: viewers,
    is_live: isLive,
    provider,
    key: `${provider}:${login}`,
    watch_url: '',
  }) as ProviderStreamRow;

describe('mergeFollowedLive', () => {
  it('keeps Twitch xqc and Kick xqc as two rows in their own key spaces', () => {
    const out = mergeFollowedLive([tw('xqc', 10)], { 'kick:xqc': row('kick', 'xqc', 5) });
    expect(out.map((s) => `${s.provider ?? 'twitch'}:${s.user_login}`)).toEqual(['twitch:xqc', 'kick:xqc']);
  });

  it('drops Kick rows that are not live', () => {
    const out = mergeFollowedLive([tw('a', 1)], { 'kick:b': row('kick', 'b', 9, false) });
    expect(out).toHaveLength(1);
  });

  it('leaves out platforms the phone cannot watch', () => {
    const out = mergeFollowedLive([tw('a', 1)], { 'youtube:b': row('youtube', 'b', 9) });
    expect(out).toHaveLength(1);
  });

  it('returns the Twitch list itself when there is nothing to add', () => {
    const list = [tw('a', 1)];
    expect(mergeFollowedLive(list, {})).toBe(list);
  });

  it('orders the merged list by viewers', () => {
    const out = mergeFollowedLive([tw('big', 100), tw('small', 1)], { 'kick:mid': row('kick', 'mid', 50) });
    expect(out.map((s) => s.user_login)).toEqual(['big', 'mid', 'small']);
  });
});
