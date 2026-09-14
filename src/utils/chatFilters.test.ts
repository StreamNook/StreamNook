import { describe, expect, it } from 'vitest';
import {
  filterChannelKey,
  isFilteredChatUser,
  isHiddenInScope,
  normalizeFilterName,
  withHiddenUser,
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

describe('chatFilters writers', () => {
  it('adds and removes a global name idempotently', () => {
    let cf = withHiddenUser(undefined, '@SpamBot', 'global', true);
    expect(isHiddenInScope(cf, 'spambot', 'global')).toBe(true);
    cf = withHiddenUser(cf, 'SPAMBOT', 'global', true);
    expect(cf.hidden_users).toEqual(['spambot']);
    cf = withHiddenUser(cf, 'spambot', 'global', false);
    expect(isHiddenInScope(cf, 'spambot', 'global')).toBe(false);
  });

  it('per-channel add targets one composite key and cleans up empty lists', () => {
    const scope = { provider: 'twitch' as const, channel: 'TheBurntPeanut' };
    let cf = withHiddenUser(undefined, 'streamelements', scope, true);
    expect(Object.keys(cf.per_channel ?? {})).toEqual(['twitch:theburntpeanut']);
    expect(isHiddenInScope(cf, 'streamelements', scope)).toBe(true);
    expect(isHiddenInScope(cf, 'streamelements', 'global')).toBe(false);
    cf = withHiddenUser(cf, 'streamelements', scope, false);
    expect(cf.per_channel).toEqual({});
  });

  it('normalization strips @ and folds case', () => {
    expect(normalizeFilterName('  @@Nightbot ')).toBe('nightbot');
  });
});
