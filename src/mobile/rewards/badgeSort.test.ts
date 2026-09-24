import { describe, it, expect } from 'vitest';
import { orderBadges } from './badgeSort';
import type { GlobalBadge } from './badgeGalleryStore';

const UNRANKED = Number.MAX_SAFE_INTEGER;
const DAY = 86_400_000;
const badge = (key: string, position: number, addedMs: number, extra: Partial<GlobalBadge> = {}): GlobalBadge => ({
  key,
  setId: key,
  versionId: '1',
  title: key,
  description: '',
  image: '',
  position,
  addedMs,
  usage: 0,
  status: null,
  dateInfo: '',
  moreInfo: '',
  infoUrl: '',
  ...extra,
});

// Mostly ranked badges plus the two kinds the old mixed comparator tripped on:
// a brand-new unranked badge (a pushed one, dated by its campaign) and one with
// no metadata at all.
const input = [
  badge('mid', 1, 20 * DAY),
  badge('undated', UNRANKED, 0),
  badge('oldest', 3, 1 * DAY),
  badge('fresh', UNRANKED, 40 * DAY),
  badge('new-b', 0, 30 * DAY, { usage: 5 }),
  badge('old', 2, 10 * DAY),
  badge('new-a', UNRANKED, 30 * DAY),
];
const keys = (list: GlobalBadge[]) => list.map((b) => b.key);

describe('badge gallery order', () => {
  it('newest runs strictly by date, rank breaking same-day ties, undated last', () => {
    expect(keys(orderBadges(input, 'newest'))).toEqual(['fresh', 'new-b', 'new-a', 'mid', 'old', 'oldest', 'undated']);
  });

  it('does not depend on the order the input arrives in', () => {
    expect(keys(orderBadges([...input].reverse(), 'newest'))).toEqual(keys(orderBadges(input, 'newest')));
  });

  it('oldest is exactly newest reversed', () => {
    expect(keys(orderBadges(input, 'oldest'))).toEqual(keys(orderBadges(input, 'newest')).reverse());
  });

  it('available and coming soon lead with their group, each group still newest first', () => {
    const tagged = input.map((b) =>
      b.key === 'oldest' || b.key === 'mid'
        ? { ...b, status: 'available' as const }
        : b.key === 'old' || b.key === 'fresh'
          ? { ...b, status: 'coming-soon' as const }
          : b,
    );
    expect(keys(orderBadges(tagged, 'available'))).toEqual(['mid', 'oldest', 'fresh', 'new-b', 'new-a', 'old', 'undated']);
    expect(keys(orderBadges(tagged, 'soon'))).toEqual(['fresh', 'old', 'new-b', 'new-a', 'mid', 'oldest', 'undated']);
  });

  it('most used breaks ties newest first', () => {
    expect(keys(orderBadges(input, 'usage'))).toEqual(['new-b', 'fresh', 'new-a', 'mid', 'old', 'oldest', 'undated']);
  });
});
