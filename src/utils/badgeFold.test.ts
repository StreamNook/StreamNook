import { describe, expect, it } from 'vitest';
import rules from './badgeFold.rules.json';
import { badgeFoldLimit, foldBadges, foldKind, rarityTier, type FoldKind } from './badgeFold';

describe('foldBadges', () => {
  it('runs the shared case table', () => {
    expect(rules.cases.length).toBeGreaterThanOrEqual(12);
    for (const c of rules.cases) {
      expect(foldBadges(c.kinds as FoldKind[], c.limit), c.name).toEqual({ shown: c.shown, hidden: c.hidden, tier: c.tier });
    }
  });
  it('starts the tiers at 2, 3, 5 and 8 add-ons, like Rust', () => {
    expect([0, 1, 2, 3, 4, 5, 6, 7, 8, 9].map(rarityTier)).toEqual([0, 0, 1, 2, 2, 3, 3, 3, 4, 4]);
  });
  it('names kinds like Rust', () => {
    expect(foldKind('twitch', 'moderator')).toBe('role');
    expect(foldKind('kick', 'subscriber')).toBe('platform');
    expect(foldKind('youtube', 'owner')).toBe('role');
    expect(foldKind('7tv', '')).toBe('seventv');
    expect(foldKind('limerino', '')).toBe('addon');
    expect(foldKind('streamnook', '')).toBe('streamnook');
  });
  it('reads the setting as the shared limit cases say, like Rust', () => {
    expect(rules.limit_cases.length).toBeGreaterThanOrEqual(8);
    for (const c of rules.limit_cases) {
      expect(badgeFoldLimit(c.raw ?? undefined), String(c.raw)).toBe(c.limit);
    }
  });
});
