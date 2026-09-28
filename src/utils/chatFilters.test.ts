import { describe, expect, it } from 'vitest';
import {
  filterChannelKey,
  isFilteredChatUser,
  isHiddenInScope,
  normalizeFilterName,
} from './chatFilters';
import type { ChatFilterSettings } from '../types';

describe('chatFilters matching', () => {
  it('matches the overlay dialect: case-insensitive, @ stripped, login or display name', () => {
    const cf: ChatFilterSettings = { hidden_users: ['@StreamElements'] };
    expect(isFilteredChatUser(cf, 'twitch', 'theburntpeanut', 'streamelements')).toBe(true);
    expect(isFilteredChatUser(cf, 'twitch', 'theburntpeanut', 'someone', 'StreamElements')).toBe(true);
    expect(isFilteredChatUser(cf, 'kick', 'other', 'STREAMELEMENTS')).toBe(true);
    expect(isFilteredChatUser(cf, 'twitch', 'theburntpeanut', 'streamelement')).toBe(false);
  });

  it('per-channel entries hide only in their channel', () => {
    const cf: ChatFilterSettings = {
      per_channel: { [filterChannelKey('twitch', 'theburntpeanut')]: ['streamelements'] },
    };
    expect(isFilteredChatUser(cf, 'twitch', 'theburntpeanut', 'streamelements')).toBe(true);
    expect(isFilteredChatUser(cf, 'twitch', 'xqc', 'streamelements')).toBe(false);
    expect(isFilteredChatUser(cf, 'kick', 'theburntpeanut', 'streamelements')).toBe(false);
  });

  it('a bare legacy channel key still resolves as Twitch', () => {
    const cf: ChatFilterSettings = { per_channel: { theburntpeanut: ['spambot'] } };
    expect(isFilteredChatUser(cf, 'twitch', 'theburntpeanut', 'spambot')).toBe(true);
  });

  it('hide_bots catches the known-bot list in every channel', () => {
    const cf: ChatFilterSettings = { hide_bots: true };
    expect(isFilteredChatUser(cf, 'twitch', 'anychannel', 'nightbot')).toBe(true);
    expect(isFilteredChatUser(cf, 'twitch', 'anychannel', 'streamelements')).toBe(true);
    expect(isFilteredChatUser(cf, 'twitch', 'anychannel', 'a_human')).toBe(false);
    expect(isFilteredChatUser({ hide_bots: false }, 'twitch', 'anychannel', 'nightbot')).toBe(false);
  });

  it('empty settings filter nothing and stay cheap', () => {
    expect(isFilteredChatUser(undefined, 'twitch', 'c', 'anyone')).toBe(false);
    expect(isFilteredChatUser({}, 'twitch', 'c', 'anyone')).toBe(false);
  });
});

describe('chatFilters scope reads', () => {
  it('reads a name hidden everywhere or in one channel', () => {
    const scope = { provider: 'twitch' as const, channel: 'TheBurntPeanut' };
    const cf: ChatFilterSettings = {
      hidden_users: ['spambot'],
      per_channel: { 'twitch:theburntpeanut': ['streamelements'], xqc: ['nightbot'] },
    };
    expect(isHiddenInScope(cf, '@SpamBot', 'global')).toBe(true);
    expect(isHiddenInScope(cf, 'streamelements', scope)).toBe(true);
    expect(isHiddenInScope(cf, 'streamelements', 'global')).toBe(false);
    expect(isHiddenInScope(cf, 'nightbot', { provider: 'twitch', channel: 'xqc' })).toBe(true);
  });

  it('normalization strips @ and folds case', () => {
    expect(normalizeFilterName('  @@Nightbot ')).toBe('nightbot');
  });
});
