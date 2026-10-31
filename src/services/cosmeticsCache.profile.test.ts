// getFullProfileWithFallback: whether 7TV answered travels with the profile,
// and a profile 7TV failed for is not kept for the session.
import { beforeEach, describe, expect, it, vi } from 'vitest';

const badgeCalls = { n: 0 };
vi.mock('./badgeService', () => ({
  getAllUserBadgesWithEarned: async () => {
    badgeCalls.n += 1;
    return { displayBadges: [], earnedBadges: [], thirdPartyBadges: [] };
  },
}));

let inventory: unknown = null;
let worn: { data: { paints: unknown[]; badges: unknown[] }; hardFail: boolean } = { data: { paints: [], badges: [] }, hardFail: false };
vi.mock('./seventvService', () => ({
  fetchUserInventory: async () => inventory,
  getUserCosmetics: async () => worn,
}));

import { clearCosmeticsMemoryCache, getFullProfileWithFallback } from './cosmeticsCache';

const PAINTS = [{ id: 'p1', selected: true }, { id: 'p2' }];

beforeEach(() => {
  clearCosmeticsMemoryCache();
  badgeCalls.n = 0;
  inventory = null;
  worn = { data: { paints: [], badges: [] }, hardFail: false };
});

describe('getFullProfileWithFallback answer flags', () => {
  it('a failed 7TV read is marked unanswered and not kept, so a reopen after recovery is answered', async () => {
    worn = { data: { paints: [], badges: [] }, hardFail: true };
    const failed = await getFullProfileWithFallback('1', 'one');
    expect(failed.inventoryAnswered).toBe(false);
    expect(failed.wornAnswered).toBe(false);

    // 7TV recovers: the reopen fetches again instead of serving the failed copy.
    inventory = { paints: PAINTS, badges: [] };
    const again = await getFullProfileWithFallback('1', 'one');
    expect(badgeCalls.n).toBe(2);
    expect(again.inventoryAnswered).toBe(true);
    expect(again.wornAnswered).toBe(true);
    expect(again.seventvCosmetics.paints).toHaveLength(2);

    // An answered profile is kept.
    await getFullProfileWithFallback('1', 'one');
    expect(badgeCalls.n).toBe(2);
  });

  it('a failed inventory with a real worn answer: the paint is an answer, the counts are not', async () => {
    worn = { data: { paints: [PAINTS[0]], badges: [] }, hardFail: false };
    const profile = await getFullProfileWithFallback('2', 'two');
    expect(profile.inventoryAnswered).toBe(false);
    expect(profile.wornAnswered).toBe(true);
    // Kept, with its flags: a reopen still knows the counts did not answer.
    const reopened = await getFullProfileWithFallback('2', 'two');
    expect(badgeCalls.n).toBe(1);
    expect(reopened.inventoryAnswered).toBe(false);
  });
});
