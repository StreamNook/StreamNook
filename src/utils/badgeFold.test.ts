import { describe, expect, it } from 'vitest';
import rules from './badgeFold.rules.json';
import { badgeFoldLimit, foldBadges, foldKind, type FoldKind } from './badgeFold';

describe('foldBadges', () => {
  it('runs the shared case table', () => {
    expect(rules.cases.length).toBeGreaterThanOrEqual(12);
    for (const c of rules.cases) {
      expect(foldBadges(c.kinds as FoldKind[], c.limit), c.name).toEqual({ shown: c.shown, hidden: c.hidden });
    }
  });
  it('names kinds like Rust', () => {
    expect(foldKind('twitch', 'moderator')).toBe('role');
    expect(foldKind('kick', 'subscriber')).toBe('platform');
    expect(foldKind('youtube', 'owner')).toBe('role');
    expect(foldKind('7tv', '')).toBe('seventv');
    expect(foldKind('limerino', '')).toBe('addon');
    expect(foldKind('streamnook', '')).toBe('streamnook');
  });
  it('reads the setting', () => {
    expect(badgeFoldLimit(undefined)).toBe(3);
    expect(badgeFoldLimit(0)).toBe(0);
    expect(badgeFoldLimit(1)).toBe(2);
    expect(badgeFoldLimit(9)).toBe(5);
  });
});
