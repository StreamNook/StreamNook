// The atmosphere ownership gate lives in Rust (services/cosmetics_registry.rs
// `may_wear_atmosphere`, applied by services/member_theme.rs before a theme ever
// reaches a page), and its rules are tested there. What stays here is the seam
// between the two key spaces: chat files a row under a CHAT key, while every
// theme is filed under the member's TWITCH id. If the chat key reached Rust
// instead of the member id, a Kick or YouTube member would read as owning
// nothing and be silently blanked on the platforms this feature exists for.
//
// So these assert WHICH id Rust was asked about, not merely the outcome: the
// outcome is identical on Twitch either way, which is what would let it ship.

import { describe, it, expect, vi, beforeEach } from 'vitest';

const state = {
  /** Every member id `get_member_theme` was asked about, in order. */
  askedFor: [] as string[],
  /** What Rust answers for a member. */
  atmosphereId: null as string | null,
};

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string, args?: { memberId?: string }) => {
    if (cmd !== 'get_member_theme') return null;
    state.askedFor.push(args?.memberId ?? '');
    return { memberId: args?.memberId, theme: state.atmosphereId, atmosphereId: state.atmosphereId, hiddenInChat: false };
  }),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

vi.mock('../services/supabaseService', () => ({
  notifyMemberIdentityChanged: () => {},
  isStreamNookUser: () => true,
  subscribeAtmospheresVersion: () => () => {},
  subscribeStreamNookRegistryVersion: () => () => {},
}));

vi.mock('../services/atmospheres', () => ({ getAtmosphere: () => null }));

// Repo convention (see multiNookIdentity.test.ts): tests run in the node
// environment, so anything that touches the backend or the app store at import
// time is mocked out.
vi.mock('./AppStore', () => {
  const s = { addToast: vi.fn(), settings: {}, currentStream: null, updateSettings: vi.fn() };
  return { useAppStore: Object.assign(vi.fn(), { getState: () => s, setState: vi.fn(), subscribe: vi.fn() }) };
});
vi.mock('../services/cosmeticsCache', () => ({
  getCosmeticsFromMemoryCache: () => null,
  getCosmeticsWithFallback: async () => null,
  isUserCosmeticsHardFailed: () => false,
  isUserCosmeticsUsable: () => true,
  isUserCosmeticsRecheckDue: () => false,
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

const { ensureAtmosphereResolved, useChatUserStore } = await import('./chatUserStore');
const { setMemberAliases, __resetMemberAliases } = await import('../utils/memberIdentity');

const SUBSCRIBER_ATM = 'aurora';
const MEMBER = '249031143';
const KICK_ROW = 'kick:12345';

const settle = async () => {
  for (let i = 0; i < 6; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  for (let i = 0; i < 6; i++) await Promise.resolve();
};

beforeEach(() => {
  state.askedFor = [];
  state.atmosphereId = SUBSCRIBER_ATM;
  __resetMemberAliases();
  useChatUserStore.getState().clearUsers();
});

describe('member themes across platforms', () => {
  it('asks Rust about the MEMBER, not the chat row, for a linked Kick chatter', async () => {
    setMemberAliases(new Map([[KICK_ROW, MEMBER]]));
    ensureAtmosphereResolved(KICK_ROW);
    await settle();

    expect(state.askedFor).toContain(MEMBER);
    expect(state.askedFor).not.toContain(KICK_ROW);
  });

  it('paints the answer onto their Kick row', async () => {
    setMemberAliases(new Map([[KICK_ROW, MEMBER]]));
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
    ensureAtmosphereResolved(KICK_ROW);
    await settle();

    expect(state.askedFor).toEqual([]);
  });

  it('paints what Rust decided, including a refusal', async () => {
    // Rust answers null for an atmosphere the member may not wear; the row must
    // show nothing rather than re-deriving the pick.
    // A member no earlier test resolved (themes are cached per member).
    const REFUSED = '1005';
    state.atmosphereId = null;
    useChatUserStore.getState().addUser({ userId: REFUSED, username: 'm', displayName: 'M', color: '#fff' });
    ensureAtmosphereResolved(REFUSED);
    await settle();

    expect(state.askedFor).toEqual([REFUSED]);
    expect(useChatUserStore.getState().users.get(REFUSED)?.atmosphereId ?? null).toBeNull();
  });
});
