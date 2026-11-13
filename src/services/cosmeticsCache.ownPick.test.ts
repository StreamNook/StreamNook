// Picking your own 7TV paint or badge flips it in place at once. Chat's cache
// holds only what you WEAR (the chat path never reads the whole inventory), so
// the new pick is not in it; the pick must still show on your name, and the
// profile cache must keep every owned item with the mark moved.
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('./badgeService', () => ({
  getAllUserBadgesWithEarned: async () => ({ displayBadges: [], earnedBadges: [], thirdPartyBadges: [] }),
}));

const OLD = { id: 'p-old', name: 'Old', data: { layers: [], shadows: [] } };
const NEW = { id: 'p-new', name: 'New', data: { layers: [], shadows: [] } };
const BADGE_OLD = { id: 'b-old', name: 'Old badge' };
const BADGE_NEW = { id: 'b-new', name: 'New badge' };

// Rust's answers: the worn lookup (chat) and the owned inventory (profile).
let worn: { data: { paints: unknown[]; badges: unknown[] }; hardFail: boolean };
let inventory: { paints: unknown[]; badges: unknown[] } | null;
const calls = { worn: 0, invalidate: 0 };
vi.mock('./seventvService', () => ({
  fetchUserInventory: async () => inventory,
  getUserCosmetics: async () => {
    calls.worn += 1;
    return worn;
  },
  invalidateUserCosmeticsCache: async () => {
    calls.invalidate += 1;
  },
}));

import {
  applyLocalCosmeticSelection,
  clearCosmeticsMemoryCache,
  getCosmeticsFromMemoryCache,
  getCosmeticsWithFallback,
  getFullProfileWithFallback,
  getProfileFromMemoryCache,
  subscribeToCosmetics,
  wearInList,
} from './cosmeticsCache';

const wornPaint = (c: { paints: any[] } | null | undefined) => c?.paints.find((p) => p.selected)?.id ?? null;
const wornBadge = (c: { badges: any[] } | null | undefined) => c?.badges.find((b) => b.selected)?.id ?? null;

beforeEach(async () => {
  clearCosmeticsMemoryCache();
  calls.worn = 0;
  calls.invalidate = 0;
  worn = { data: { paints: [{ ...OLD, selected: true }], badges: [{ ...BADGE_OLD, selected: true }] }, hardFail: false };
  inventory = {
    paints: [{ ...OLD, selected: true }, NEW],
    badges: [{ ...BADGE_OLD, selected: true }, BADGE_NEW],
  };
  // Chat resolved you (worn only), then the picker read your inventory.
  await getCosmeticsWithFallback('me');
  await getFullProfileWithFallback('me', 'me');
});

describe('applyLocalCosmeticSelection on your own account', () => {
  it('a paint missing from the worn-only chat entry still shows on your name at once', () => {
    const seen: (string | null)[] = [];
    const off = subscribeToCosmetics((id, c) => id === 'me' && seen.push(wornPaint(c)));
    expect(applyLocalCosmeticSelection('me', { paintId: NEW.id, paint: NEW })).toBe(true);
    off();
    expect(seen).toEqual([NEW.id]);
    expect(wornPaint(getCosmeticsFromMemoryCache('me'))).toBe(NEW.id);
  });

  it('the profile cache keeps every owned paint and only moves the mark', () => {
    applyLocalCosmeticSelection('me', { paintId: NEW.id, paint: NEW });
    const owned = getProfileFromMemoryCache('me')!.seventvCosmetics;
    expect(owned.paints.map((p: any) => p.id)).toEqual([OLD.id, NEW.id]);
    expect(wornPaint(owned)).toBe(NEW.id);
    // Badges were not part of the change.
    expect(wornBadge(owned)).toBe(BADGE_OLD.id);
  });

  it('without a definition from the caller, the one in the owned inventory is used', () => {
    expect(applyLocalCosmeticSelection('me', { badgeId: BADGE_NEW.id })).toBe(true);
    expect(wornBadge(getCosmeticsFromMemoryCache('me'))).toBe(BADGE_NEW.id);
    expect(wornPaint(getCosmeticsFromMemoryCache('me'))).toBe(OLD.id);
  });

  it('taking the paint off leaves nothing worn in either cache', () => {
    applyLocalCosmeticSelection('me', { paintId: null });
    expect(wornPaint(getCosmeticsFromMemoryCache('me'))).toBeNull();
    const owned = getProfileFromMemoryCache('me')!.seventvCosmetics;
    expect(owned.paints).toHaveLength(2);
    expect(wornPaint(owned)).toBeNull();
  });

  it('a pick known nowhere re-reads your cosmetics instead of drawing a bare name', async () => {
    const before = calls.worn;
    worn = { data: { paints: [{ id: 'p-unknown', name: 'X', data: { layers: [], shadows: [] }, selected: true }], badges: [] }, hardFail: false };
    expect(applyLocalCosmeticSelection('me', { paintId: 'p-unknown' })).toBe(false);
    // The cached paint stays on screen until the re-read answers.
    expect(wornPaint(getCosmeticsFromMemoryCache('me'))).toBe(OLD.id);
    await vi.waitFor(() => expect(wornPaint(getCosmeticsFromMemoryCache('me'))).toBe('p-unknown'));
    expect(calls.worn).toBe(before + 1);
    expect(calls.invalidate).toBe(1);
  });
});

describe('wearInList', () => {
  it('marks only the chosen item and puts a missing one first', () => {
    const list = [{ id: 'a', selected: true }, { id: 'b' }];
    expect(wearInList(list, 'b', null).map((i) => i.selected)).toEqual([false, true]);
    expect(wearInList(list, 'c', { id: 'c' }).map((i) => [i.id, i.selected])).toEqual([
      ['c', true],
      ['a', false],
      ['b', false],
    ]);
    expect(wearInList(list, null, null).some((i) => i.selected)).toBe(false);
  });
});
