import { describe, expect, it } from 'vitest';
import { makeKey, normalizeChannel, parseKey } from './providerKey';

describe('providerKey', () => {
  it('lowercases case-insensitive platforms', () => {
    expect(makeKey('twitch', 'NickMercs')).toBe('twitch:nickmercs');
    expect(makeKey('kick', 'Trainwreck')).toBe('kick:trainwreck');
  });

  it('never normalises YouTube ids (a video id is case-sensitive)', () => {
    expect(makeKey('youtube', 'AGr94tpNVkw')).toBe('youtube:AGr94tpNVkw');
    expect(normalizeChannel('youtube', 'AGr94tpNVkw')).toBe('AGr94tpNVkw');
  });

  it('round-trips through parseKey', () => {
    const parsed = parseKey(makeKey('kick', 'SomeSlug'));
    expect(parsed).toEqual({ provider: 'kick', channel: 'someslug' });
  });

  it('treats a bare key as a legacy Twitch login', () => {
    expect(parseKey('nickmercs')).toEqual({ provider: 'twitch', channel: 'nickmercs' });
  });

  it('a Kick and a Twitch channel sharing a login stay distinct keys', () => {
    expect(makeKey('kick', 'shared')).not.toBe(makeKey('twitch', 'shared'));
  });
});

// Edge cases around the bare-key rule. Every persisted grid, preset, favorite
// and chat slice written before providers existed holds a bare login, so these
// paths stay live indefinitely, not just through one migration.
describe('providerKey edges', () => {
  it('reads text that merely contains a colon as a Twitch login, not a provider', () => {
    expect(parseKey('notaprovider:thing').provider).toBe('twitch');
    expect(parseKey('12:34')).toEqual({ provider: 'twitch', channel: '12:34' });
  });

  it('folds case on a bare login so legacy keys of any casing converge', () => {
    expect(parseKey('NickMercs')).toEqual({ provider: 'twitch', channel: 'nickmercs' });
  });

  it('round-trips a composite YouTube key without touching its casing', () => {
    const key = makeKey('youtube', 'AGr94tpNVkw');
    const back = parseKey(key);
    expect(back.channel).toBe('AGr94tpNVkw');
    expect(makeKey(back.provider, back.channel)).toBe(key);
  });

  it('keeps two YouTube ids that differ only by case as different channels', () => {
    expect(makeKey('youtube', 'AGr94tpNVkw')).not.toBe(makeKey('youtube', 'agr94tpnvkw'));
  });
});
