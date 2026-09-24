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
  /** Every id the ownership gate was asked about, in order. */
  ownershipAskedFor: [] as string[],
  /** What the member's stored profile theme resolves to. */
  profileTheme: 'tier' as string | null,
};

vi.mock('../services/supabaseService', () => ({
  isCosmeticsRegistryLoaded: () => state.loaded,
  getOwnedCosmeticSlugs: (id: string) => {
    state.ownershipAskedFor.push(id);
    return state.owned.get(id) ?? new Set<string>();
  },
  notifyMemberIdentityChanged: () => {},
  // Unused by the gate, but imported by the module under test.
  isStreamNookUser: () => true,
  getProfilePrefs: async () => ({ profileTheme: state.profileTheme, hiddenSections: [] }),
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

const { mayWearAtmosphere, registerOwnAtmospheres, ensureAtmosphereResolved, useChatUserStore } =
  await import('./chatUserStore');
const { setMemberAliases, __resetMemberAliases } = await import('../utils/memberIdentity');

const SUBSCRIBER_ATM = 'aurora';
const ACCOLADE_ATM = 'midnight';
const SUBSCRIBER_BADGE = 'streamnook-subscriber';

beforeEach(() => {
  state.loaded = true;
  state.owned = new Map();
  state.ownershipAskedFor = [];
  state.profileTheme = 'tier';
  __resetMemberAliases();
  useChatUserStore.getState().clearUsers();
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

// ── The cross-platform seam ──────────────────────────────────────────────────
//
// Everything the gate reads is filed under a TWITCH user id. Chat identifies a
// Kick or YouTube chatter by that platform's own id. If the chat key reaches the
// gate instead of the member id, `getOwnedCosmeticSlugs` looks up a key that
// cannot exist, finds nothing, and a paying member is silently blanked on their
// own row — on the exact platforms this feature exists to cover.
//
// So these assert WHICH id the gate was asked about, not merely the outcome: the
// outcome is identical on Twitch either way, which is what would have let this
// ship unnoticed.
describe('ownership across platforms', () => {
  const MEMBER = '249031143';
  const KICK_ROW = 'kick:12345';

  const settle = async () => {
    for (let i = 0; i < 6; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
    for (let i = 0; i < 6; i++) await Promise.resolve();
  };

  it('asks about the MEMBER, not the chat row, for a linked Kick chatter', async () => {
    setMemberAliases(new Map([[KICK_ROW, MEMBER]]));
    state.owned.set(MEMBER, new Set([SUBSCRIBER_BADGE]));
    state.profileTheme = SUBSCRIBER_ATM;

    ensureAtmosphereResolved(KICK_ROW);
    await settle();

    expect(state.ownershipAskedFor).toContain(MEMBER);
    expect(state.ownershipAskedFor).not.toContain(KICK_ROW);
  });

  it('paints a subscriber’s atmosphere onto their Kick row', async () => {
    // The user-visible half of the same thing: handed the chat key, the gate
    // would see no owned slugs and store null here instead.
    setMemberAliases(new Map([[KICK_ROW, MEMBER]]));
    state.owned.set(MEMBER, new Set([SUBSCRIBER_BADGE]));
    state.profileTheme = SUBSCRIBER_ATM;

    useChatUserStore.getState().addUser({
      userId: KICK_ROW,
      username: 'someone',
      displayName: 'Someone',
      color: '#fff',
    });
    ensureAtmosphereResolved(KICK_ROW);
    await settle();

    expect(useChatUserStore.getState().users.get(KICK_ROW)?.atmosphereId).toBe(SUBSCRIBER_ATM);
  });

  it('does nothing for an unclaimed chatter rather than guessing', async () => {
    // No alias: the id is a Kick id and must never be read as a Twitch one, even
    // though both platforms number their users the same way.
    state.owned.set(MEMBER, new Set([SUBSCRIBER_BADGE]));
    state.profileTheme = SUBSCRIBER_ATM;

    ensureAtmosphereResolved(KICK_ROW);
    await settle();

    expect(state.ownershipAskedFor).toEqual([]);
  });
});
