import { describe, expect, it } from 'vitest';
import { hiddenStats, isStatHidden, PROFILE_VISIBILITY_KEYS, storedKeys, toggleStat } from './profileVisibility';

describe('profile visibility', () => {
  it('reads an older section key as the whole section hidden', () => {
    const stored = ['lifetime'];
    expect(isStatHidden(stored, 'messages')).toBe(true);
    expect(isStatHidden(stored, 'streams')).toBe(true);
    expect(isStatHidden(stored, 'favorite_channel')).toBe(true);
    expect(isStatHidden(stored, 'hours')).toBe(false);
  });

  it('keeps the rest of a section hidden when one stat is shown again', () => {
    const next = toggleStat(['lifetime'], 'messages');
    expect(isStatHidden(next, 'messages')).toBe(false);
    expect(isStatHidden(next, 'streams')).toBe(true);
    expect(isStatHidden(next, 'member_rank')).toBe(true);
    // Older apps still get the section key, so they over-hide rather than show.
    expect(next).toContain('lifetime');
  });

  it('saves the section key for older apps whenever a stat in it is hidden', () => {
    const next = toggleStat([], 'followers');
    expect(next).toEqual(expect.arrayContaining(['followers', 'twitch']));
    expect(isStatHidden(next, 'followers')).toBe(true);
    expect(isStatHidden(next, 'twitch_age')).toBe(false);
    expect(isStatHidden(next, 'account_type')).toBe(false);
  });

  it('drops the section key once nothing in the section is hidden', () => {
    const hidden = toggleStat(toggleStat([], 'followers'), 'followers');
    expect(hidden).toEqual([]);
  });

  it('handles single-stat sections and new stats without a section', () => {
    expect(isStatHidden(['emotes'], 'emotes')).toBe(true);
    expect(toggleStat([], 'paints')).toEqual(['paints']);
    expect(hiddenStats(['views', 'relics'])).toEqual(new Set(['views', 'relics']));
  });

  it('accepts every stat and every older section key, nothing else', () => {
    for (const k of ['roast', 'twitch', 'lifetime', 'emotes', 'accolades', 'views', 'hours', 'paints', 'joined']) {
      expect(PROFILE_VISIBILITY_KEYS.has(k)).toBe(true);
    }
    expect(PROFILE_VISIBILITY_KEYS.has('channel_points')).toBe(false);
    expect(storedKeys(new Set())).toEqual([]);
  });
});
