// resolveWornBadges: each badge part says whether its own read answered, with
// the failure shapes the real reads produce (a marked identity fallback, a
// Twitch lookup that rejects, a BTTV lookup that could not be asked).
import { beforeEach, describe, expect, it, vi } from 'vitest';

// The browser globals the app's stores read at import (the same stand-ins
// ChatBadgeIcons.test.tsx uses).
vi.hoisted(() => {
  const store = new Map<string, string>();
  const storage = {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, String(v)),
    removeItem: (k: string) => void store.delete(k),
    clear: () => store.clear(),
    key: () => null,
    length: 0,
  };
  const noop = () => {};
  const el = () => ({ style: {}, dataset: {} as Record<string, string>, setAttribute: noop, appendChild: noop, classList: { add: noop, remove: noop, toggle: noop }, addEventListener: noop, removeEventListener: noop });
  const g = globalThis as Record<string, unknown>;
  g.localStorage = storage;
  g.sessionStorage = storage;
  g.document = { documentElement: el(), body: el(), head: el(), createElement: el, addEventListener: noop, removeEventListener: noop, querySelector: () => null, querySelectorAll: () => [], visibilityState: 'visible', fonts: { ready: Promise.resolve() } };
  g.window = Object.assign(globalThis, {
    location: { hash: '', href: 'http://localhost/', origin: 'http://localhost', search: '' },
    addEventListener: noop,
    removeEventListener: noop,
    matchMedia: () => ({ matches: false, addEventListener: noop, removeEventListener: noop, addListener: noop, removeListener: noop }),
    requestAnimationFrame: (cb: () => void) => setTimeout(cb, 0),
    cancelAnimationFrame: noop,
    __TAURI_INTERNALS__: { invoke: () => Promise.resolve(null), transformCallback: () => 0, metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } } },
  });
  if (!g.navigator) g.navigator = { userAgent: 'node', platform: 'Win32', language: 'en-US' };
});

const state = {
  resolvedFails: false,
  loadoutFails: false,
  twitchFails: false,
  bttv: 'https://cdn.betterttv.net/badges/pro/x.webp' as string | null | undefined,
  keys: ['ffz:ffz-3', 'BTTV:bttv-pro'] as string[],
};
const fallbacks = new WeakSet<object>();

vi.mock('@tauri-apps/api/core', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@tauri-apps/api/core')>()),
  invoke: async (cmd: string) => {
    if (cmd === 'get_selected_twitch_badge') {
      if (state.twitchFails) throw new Error('Twitch returned no user');
      return { src: 'https://static-cdn.jtvnw.net/badges/v1/live/3', title: 'Live' };
    }
    return null;
  },
}));
vi.mock('../../services/identityService', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../services/identityService')>()),
  getResolvedIdentity: async (id: string) => {
    const v = {
      twitch_user_id: id,
      customized: !state.resolvedFails,
      badges: state.resolvedFails ? [] : [{ key: 'ffz:ffz-3', provider: 'ffz', title: 'FFZ', image_url: 'https://cdn.frankerfacez.com/badge/3/4/rounded' }],
      seventv: null,
      updated_at: null,
    };
    if (state.resolvedFails) fallbacks.add(v);
    return v;
  },
  getIdentityWithCache: async (id: string) => {
    const v = { twitch_user_id: id, customized: true, badges: state.loadoutFails ? [] : state.keys, paint: null, updated_at: null };
    if (state.loadoutFails) fallbacks.add(v);
    return v;
  },
  isIdentityFallback: (v: unknown) => typeof v === 'object' && v !== null && fallbacks.has(v),
}));
vi.mock('../../services/bttvProBadge', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../services/bttvProBadge')>()),
  resolveBttvProUrl: async () => state.bttv,
}));

import { resolveWornBadges } from './memberProfile';

beforeEach(() => {
  state.resolvedFails = false;
  state.loadoutFails = false;
  state.twitchFails = false;
  state.bttv = 'https://cdn.betterttv.net/badges/pro/x.webp';
  state.keys = ['ffz:ffz-3', 'BTTV:bttv-pro'];
});

describe('resolveWornBadges answers', () => {
  it('every read answered', async () => {
    const r = await resolveWornBadges('5', null);
    expect(r.answers).toEqual({ twitch: true, thirdParty: true, bttv: true });
    expect(r.thirdParty).toHaveLength(1);
    expect(r.twitch?.title).toBe('Live');
    expect(r.bttvPro?.src).toContain('betterttv');
  });

  it('a failed resolved identity read is not an answer for the third-party badges', async () => {
    state.resolvedFails = true;
    const r = await resolveWornBadges('5', null);
    expect(r.answers).toEqual({ twitch: true, thirdParty: false, bttv: true });
  });

  it('a failed loadout read is not an answer for BTTV Pro', async () => {
    state.loadoutFails = true;
    const r = await resolveWornBadges('5', null);
    expect(r.answers.bttv).toBe(false);
    expect(r.answers.thirdParty).toBe(true);
  });

  it("Twitch failing is not a Twitch answer; the loadout's copy fills the value", async () => {
    state.twitchFails = true;
    state.keys = ['twitch:abc123', 'ffz:ffz-3'];
    const r = await resolveWornBadges('5', null);
    expect(r.answers.twitch).toBe(false);
    expect(r.twitch?.src).toBe('https://static-cdn.jtvnw.net/badges/v1/abc123/3');
    expect(r.answers.thirdParty).toBe(true);
  });

  it('a BTTV lookup that could not be asked is not an answer; a real "no Pro" is', async () => {
    state.bttv = undefined;
    expect((await resolveWornBadges('5', null)).answers.bttv).toBe(false);
    state.bttv = null;
    const r = await resolveWornBadges('5', null);
    expect(r.answers.bttv).toBe(true);
    expect(r.bttvPro).toBeNull();
  });
});
