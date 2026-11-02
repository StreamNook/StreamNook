// getFullProfileWithFallback: whether 7TV answered travels with the profile,
// and a profile 7TV did not fully answer is kept only for the retry window.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const badgeCalls = { n: 0 };
vi.mock('./badgeService', () => ({
  getAllUserBadgesWithEarned: async () => {
    badgeCalls.n += 1;
    return { displayBadges: [], earnedBadges: [], thirdPartyBadges: [] };
  },
}));

// What Rust's inventory command answers: null for a failed read, an empty
// inventory (no 7TV id) for an account 7TV does not know.
let inventory: unknown = null;
let worn: { data: { paints: unknown[]; badges: unknown[]; seventvUserId?: string }; hardFail: boolean } = {
  data: { paints: [], badges: [] },
  hardFail: false,
};
const wornCalls = { n: 0 };
vi.mock('./seventvService', () => ({
  fetchUserInventory: async () => inventory,
  getUserCosmetics: async () => {
    wornCalls.n += 1;
    return worn;
  },
}));

import { clearCosmeticsMemoryCache, getCosmeticsWithFallback, getFullProfileWithFallback, ownedAfterRead } from './cosmeticsCache';

const PAINTS = [{ id: 'p1', selected: true }, { id: 'p2' }];
const START = new Date('2026-10-31T12:00:00Z').getTime();
const later = (ms: number) => vi.setSystemTime(START + ms);

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(START);
  clearCosmeticsMemoryCache();
  badgeCalls.n = 0;
  wornCalls.n = 0;
  inventory = null;
  worn = { data: { paints: [], badges: [] }, hardFail: false };
});
afterEach(() => {
  vi.useRealTimers();
});

describe('getFullProfileWithFallback answer flags', () => {
  it('a failed 7TV read is served, marked, only for the retry window; after it a recovered 7TV is read again', async () => {
    worn = { data: { paints: [], badges: [] }, hardFail: true };
    const failed = await getFullProfileWithFallback('1', 'one');
    expect(failed.inventoryAnswered).toBe(false);
    expect(failed.wornAnswered).toBe(false);

    later(10_000);
    const soon = await getFullProfileWithFallback('1', 'one');
    expect(badgeCalls.n).toBe(1);
    expect(soon.wornAnswered).toBe(false);

    // 7TV recovers; past the window the reopen reads again.
    inventory = { paints: PAINTS, badges: [] };
    later(31_000);
    const again = await getFullProfileWithFallback('1', 'one');
    expect(badgeCalls.n).toBe(2);
    expect(again.inventoryAnswered).toBe(true);
    expect(again.wornAnswered).toBe(true);
    expect(again.seventvCosmetics.paints).toHaveLength(2);

    // An answered profile is kept.
    later(10 * 60_000);
    await getFullProfileWithFallback('1', 'one');
    expect(badgeCalls.n).toBe(2);
  });

  it('a 7TV user whose inventory failed: the paint is an answer, the counts are not, and it is read again later', async () => {
    worn = { data: { paints: [PAINTS[0]], badges: [], seventvUserId: '01SEVENTV' }, hardFail: false };
    const profile = await getFullProfileWithFallback('2', 'two');
    expect(profile.inventoryAnswered).toBe(false);
    expect(profile.wornAnswered).toBe(true);
    later(31_000);
    await getFullProfileWithFallback('2', 'two');
    expect(badgeCalls.n).toBe(2);
  });

  it('an account 7TV does not know: its empty inventory is the answer, and it is kept', async () => {
    inventory = { paints: [], badges: [] };
    const profile = await getFullProfileWithFallback('3', 'three');
    expect(profile.inventoryAnswered).toBe(true);
    expect(profile.wornAnswered).toBe(true);
    later(10 * 60_000);
    await getFullProfileWithFallback('3', 'three');
    expect(badgeCalls.n).toBe(1);
  });

  it('a failed worn read served from memory still counts as failed', async () => {
    worn = { data: { paints: [], badges: [] }, hardFail: true };
    await getCosmeticsWithFallback('4');
    // Inside the retry window the worn cosmetics come from memory, not 7TV;
    // the failure travels with them.
    worn = { data: { paints: [PAINTS[0]], badges: [] }, hardFail: false };
    later(5_000);
    const profile = await getFullProfileWithFallback('4', 'four');
    expect(wornCalls.n).toBe(1);
    expect(profile.wornAnswered).toBe(false);
  });
});

describe('ownedAfterRead', () => {
  const owned = [{ id: 'p1', selected: true }, { id: 'p2' }, { id: 'p3' }];

  it('a worn-only read keeps every owned item on screen and moves the mark', () => {
    const next = ownedAfterRead(owned, [{ id: 'p2', selected: true }], false);
    expect(next.map((p) => p.id)).toEqual(['p1', 'p2', 'p3']);
    expect(next.filter((p) => p.selected).map((p) => p.id)).toEqual(['p2']);
  });

  it('a worn-only read with nothing worn clears the mark, and a worn item the list lacks joins the front', () => {
    expect(ownedAfterRead(owned, [], false).some((p) => p.selected)).toBe(false);
    expect(ownedAfterRead(owned, [{ id: 'p9', selected: true }], false).map((p) => p.id)).toEqual(['p9', 'p1', 'p2', 'p3']);
  });

  it('an answered inventory, or nothing shown yet, is taken as read', () => {
    const read = [{ id: 'p2', selected: true }];
    expect(ownedAfterRead(owned, read, true)).toBe(read);
    expect(ownedAfterRead(owned, read, undefined)).toBe(read);
    expect(ownedAfterRead([], read, false)).toBe(read);
  });
});
