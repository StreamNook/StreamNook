// The atmosphere ownership gate is the render-side half of closing H1: the
// profile-prefs table is world-writable under the bundled anon key, so anyone
// can SELECT a paid atmosphere. Rendering is the last place that can refuse.
//
// These assert the two directions that actually matter, because getting either
// wrong is a shipped bug: a non-owner must not paint, and a legitimate owner (or
// anyone at all while data is missing) must still paint.

import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { Atmosphere } from '../services/atmospheres';

const state = {
  loaded: true,
  owned: new Map<string, Set<string>>(),
  atmospheres: new Map<string, Atmosphere>(),
};

vi.mock('../services/supabaseService', () => ({
  isCosmeticsRegistryLoaded: () => state.loaded,
  getOwnedCosmeticSlugs: (id: string) => state.owned.get(id) ?? new Set<string>(),
  // Unused by the gate, but imported by the module under test.
  isStreamNookUser: () => true,
  getProfilePrefs: async () => ({ profileTheme: 'tier', hiddenSections: [] }),
  whenAtmospheresReady: async () => undefined,
  subscribeAtmospheresVersion: () => () => {},
  subscribeStreamNookRegistryVersion: () => () => {},
  subscribeToProfileThemeChanges: () => () => {},
}));

vi.mock('../services/atmospheres', () => ({
  getAtmosphere: (id: string | null | undefined) =>
    (id ? state.atmospheres.get(String(id).split('+')[0]) ?? null : null),
}));

// Repo convention (see multiNookIdentity.test.ts): tests run in the node
// environment, so anything that touches the backend or the app store at import
// time is mocked out. None of it is under test here; the gate is pure.
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(null) }));
vi.mock('./AppStore', () => {
  const s = { addToast: vi.fn(), settings: {}, currentStream: null, updateSettings: vi.fn() };
  return { useAppStore: Object.assign(vi.fn(), { getState: () => s, setState: vi.fn(), subscribe: vi.fn() }) };
});
vi.mock('../services/cosmeticsCache', () => ({
  getCosmeticsFromMemoryCache: () => null,
  getCosmeticsWithFallback: async () => null,
  isUserCosmeticsHardFailed: () => false,
  subscribeToCosmetics: () => () => {},
}));
vi.mock('../services/identityService', () => ({
  getResolvedIdentity: async () => null,
  getResolvedIdentityFromCache: () => null,
  getIdentityWithCache: async () => null,
  subscribeResolvedIdentity: () => () => {},
}));
vi.mock('../services/badgeService', () => ({ getGlobalThirdPartyBadges: () => [] }));
vi.mock('../services/bttvProBadge', () => ({
  BTTV_PRO_LOADOUT_KEY: 'bttv:pro',
  BTTV_PRO_BADGE_ID: 'bttv-pro',
  buildBttvProBadge: () => null,
  resolveBttvProUrl: async () => null,
}));
vi.mock('../services/cologneEvent', () => ({ parseCologneTheme: () => null }));
vi.mock('../utils/userChatOverrides', () => ({ snapshotOverrides: () => ({}) }));

const { mayWearAtmosphere, registerOwnAtmospheres } = await import('./chatUserStore');

const SUBSCRIBER_ATM = 'aurora';
const ACCOLADE_ATM = 'midnight';
const SUBSCRIBER_BADGE = 'streamnook-subscriber';

beforeEach(() => {
  state.loaded = true;
  state.owned = new Map();
  state.atmospheres = new Map<string, Atmosphere>([
    [SUBSCRIBER_ATM, { id: SUBSCRIBER_ATM, name: 'Aurora' } as Atmosphere],
    [
      ACCOLADE_ATM,
      { id: ACCOLADE_ATM, name: 'Midnight', unlock: { kind: 'accolade', accoladeId: 'insomniac' } } as Atmosphere,
    ],
  ]);
});

describe('mayWearAtmosphere', () => {
  it('refuses a subscriber atmosphere for a member who owns nothing', () => {
    expect(mayWearAtmosphere('stranger', SUBSCRIBER_ATM)).toBe(false);
  });

  it('allows it when that member owns the atmosphere per-item', () => {
    state.owned.set('owner', new Set([SUBSCRIBER_ATM]));
    expect(mayWearAtmosphere('owner', SUBSCRIBER_ATM)).toBe(true);
  });

  it('allows it for anyone holding the subscriber badge, even without a per-item row', () => {
    // grant_atmosphere_ownership runs on THEIR login, not the viewer's, so a real
    // subscriber can legitimately lack the per-item row when we render them.
    state.owned.set('sub', new Set([SUBSCRIBER_BADGE]));
    expect(mayWearAtmosphere('sub', SUBSCRIBER_ATM)).toBe(true);
  });

  it('lets accolade-gated atmospheres through (closed at Stage 3, not here)', () => {
    expect(mayWearAtmosphere('stranger', ACCOLADE_ATM)).toBe(true);
  });

  it('allows everything when the registry has not loaded, rather than blanking real members', () => {
    state.loaded = false;
    expect(mayWearAtmosphere('stranger', SUBSCRIBER_ATM)).toBe(true);
  });

  it('never blocks clearing', () => {
    expect(mayWearAtmosphere('stranger', null)).toBe(true);
  });

  it('resolves ownership against the base id when cologne modifiers are present', () => {
    state.atmospheres.set('cs2-major-cologne', { id: 'cs2-major-cologne', name: 'Cologne' } as Atmosphere);
    state.owned.set('coiner', new Set(['cs2-major-cologne']));
    expect(mayWearAtmosphere('coiner', 'cs2-major-cologne+coin+border')).toBe(true);
    expect(mayWearAtmosphere('stranger', 'cs2-major-cologne+coin')).toBe(false);
  });

  it('exempts our own accounts so a member’s own pick still previews', () => {
    registerOwnAtmospheres(['me']);
    expect(mayWearAtmosphere('me', SUBSCRIBER_ATM)).toBe(true);
  });

  it('allows an atmosphere the catalog does not know, since nothing paints anyway', () => {
    expect(mayWearAtmosphere('stranger', 'not-a-real-atmosphere')).toBe(true);
  });
});
