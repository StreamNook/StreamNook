import { beforeEach, describe, expect, it } from 'vitest';
import {
  __resetMemberAliases,
  addMemberAliases,
  chatKeysForMember,
  getMemberAliasVersion,
  memberIdFor,
  setMemberAliases,
} from './memberIdentity';

/**
 * These test the SEAM, not either side of it.
 *
 * Both key spaces are plain strings, so every way this can go wrong is a lookup
 * that returns the wrong thing or nothing at all, with no error anywhere. The
 * cases below are the ones that have actually bitten this codebase before: a
 * numeric id read as the wrong platform's, and a YouTube id folded to lowercase.
 */
describe('memberIdFor', () => {
  beforeEach(() => __resetMemberAliases());

  it('passes a bare Twitch id through untouched', () => {
    // The whole Twitch render path depends on this being an identity function,
    // so it must hold with no aliases loaded at all.
    expect(memberIdFor('249031143')).toBe('249031143');
  });

  it('accepts an explicitly namespaced Twitch key', () => {
    // Some surfaces build keys with makeKey rather than the bare convention.
    expect(memberIdFor('twitch:249031143')).toBe('249031143');
  });

  it('resolves a claimed Kick chatter to the member who claimed it', () => {
    setMemberAliases(new Map([['kick:12345', '249031143']]));
    expect(memberIdFor('kick:12345')).toBe('249031143');
  });

  it('returns null for an unclaimed chatter', () => {
    setMemberAliases(new Map([['kick:12345', '249031143']]));
    expect(memberIdFor('kick:99999')).toBeNull();
  });

  it('never lets a Kick id resolve as the Twitch user of the same number', () => {
    // The bug this file exists to prevent. Kick ids and Twitch ids share a
    // numeric range, so 676 is a real id on both platforms and two different
    // people. Rendering one under the other is cross-platform impersonation.
    setMemberAliases(new Map([['kick:676', '900000000']]));
    expect(memberIdFor('kick:676')).toBe('900000000');
    expect(memberIdFor('676')).toBe('676');
    expect(memberIdFor('kick:676')).not.toBe(memberIdFor('676'));
  });

  it('treats YouTube channel ids as case sensitive', () => {
    // UCabc and UCABC are different channels. Anything that lowercases on the
    // way in or out hands one channel's cosmetics to another.
    setMemberAliases(new Map([['youtube:UCabcDEF', '249031143']]));
    expect(memberIdFor('youtube:UCabcDEF')).toBe('249031143');
    expect(memberIdFor('youtube:ucabcdef')).toBeNull();
  });

  it('keeps an id containing a colon intact', () => {
    // Only the FIRST colon separates the provider; splitting on all of them
    // would truncate any id that happens to contain one.
    setMemberAliases(new Map([['kick:a:b', '249031143']]));
    expect(memberIdFor('kick:a:b')).toBe('249031143');
  });

  it('is null for empty and missing input rather than throwing', () => {
    expect(memberIdFor(undefined)).toBeNull();
    expect(memberIdFor(null)).toBeNull();
    expect(memberIdFor('')).toBeNull();
  });

  it('does not resolve a platform with no claims', () => {
    setMemberAliases(new Map([['kick:12345', '249031143']]));
    expect(memberIdFor('youtube:UCabcDEF')).toBeNull();
    expect(memberIdFor('tiktok:12345')).toBeNull();
  });
});

describe('alias bookkeeping', () => {
  beforeEach(() => __resetMemberAliases());

  it('drops a released claim on replace', () => {
    // A member who disconnects Kick must STOP wearing cosmetics there. A merge
    // could never express a removal, which is why the setter replaces.
    setMemberAliases(new Map([['kick:12345', '249031143']]));
    setMemberAliases(new Map());
    expect(memberIdFor('kick:12345')).toBeNull();
  });

  it('merges new entries without dropping known ones', () => {
    setMemberAliases(new Map([['kick:12345', '249031143']]));
    addMemberAliases([['youtube:UCabcDEF', '77492896']]);
    expect(memberIdFor('kick:12345')).toBe('249031143');
    expect(memberIdFor('youtube:UCabcDEF')).toBe('77492896');
  });

  it('bumps the version only when something actually changed', () => {
    // Chat resolves in waves and most answers repeat. A version bump is what
    // re-renders every mounted row, so an unchanged merge must not cause one.
    setMemberAliases(new Map([['kick:12345', '249031143']]));
    const before = getMemberAliasVersion();
    addMemberAliases([['kick:12345', '249031143']]);
    expect(getMemberAliasVersion()).toBe(before);
    addMemberAliases([['kick:12345', '77492896']]);
    expect(getMemberAliasVersion()).toBeGreaterThan(before);
  });
});

describe('chatKeysForMember', () => {
  beforeEach(() => __resetMemberAliases());

  it('finds every platform a member is claimed on', () => {
    // A theme change arrives keyed by Twitch id while the member may be on
    // screen only as kick:12345, so the update needs the reverse direction.
    setMemberAliases(
      new Map([
        ['kick:12345', '249031143'],
        ['youtube:UCabcDEF', '249031143'],
        ['kick:99999', '77492896'],
      ]),
    );
    expect(chatKeysForMember('249031143').sort()).toEqual(['kick:12345', 'youtube:UCabcDEF']);
  });

  it('is empty for a member claimed nowhere', () => {
    setMemberAliases(new Map([['kick:12345', '249031143']]));
    expect(chatKeysForMember('77492896')).toEqual([]);
  });
});
