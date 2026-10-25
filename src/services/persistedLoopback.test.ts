import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => `asset://localhost/${encodeURIComponent(p)}`,
}));

const LOOPBACK = 'http://127.0.0.1:5000/cache/t/cosmetics/';

function memoryStorage() {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => {
      m.set(k, v);
    },
    removeItem: (k: string) => {
      m.delete(k);
    },
  };
}

afterEach(() => vi.unstubAllGlobals());

describe('saved caches never keep a loopback URL', () => {
  it('own cosmetics', async () => {
    const storage = memoryStorage();
    storage.setItem(
      'streamnook_own_cosmetics_v1',
      JSON.stringify({
        u1: {
          paints: [{ id: 'p', data: { layers: [{ ty: { images: [{ url: 'https://cdn.7tv.app/p', localUrl: `${LOOPBACK}p.webp` }] } }] } }],
          badges: [{ id: 'b', localUrl: `${LOOPBACK}b.webp` }],
        },
      }),
    );
    vi.stubGlobal('localStorage', storage);
    const { registerOwnCosmeticAccounts } = await import('./cosmeticsCache');
    // Hydrates u1 from storage and writes it back through persistOwnCosmetics.
    registerOwnCosmeticAccounts(['u1']);
    const saved = storage.getItem('streamnook_own_cosmetics_v1') ?? '';
    expect(saved).toContain('"b"');
    expect(saved).toContain('https://cdn.7tv.app/p');
    expect(saved).not.toContain('127.0.0.1');
  });

  it('own profile snapshot', async () => {
    const storage = memoryStorage();
    vi.stubGlobal('localStorage', storage);
    const { writeOwnProfileCache } = await import('./ownProfileCache');
    writeOwnProfileCache('u1', {
      seventvPaint: null,
      seventvBadges: [{ id: 'b', localUrl: `${LOOPBACK}b.webp` }],
      allSeventvPaints: [],
      seventvUserId: null,
      twitchBadges: [{ id: 't', localUrl: 'asset://localhost/C%3A%5Cx%5Cbadges%5Ct.bin' }],
      thirdPartyBadges: [],
      chatIdentityBadges: [],
      selfBttvProBadge: null,
      loadout: { customized: false, badges: [] },
      profileTheme: 'default',
      hiddenSections: [],
    });
    const saved = storage.getItem('streamnook_own_profile_v1') ?? '';
    expect(saved).toContain('"b"');
    expect(saved).toContain('asset://localhost');
    expect(saved).not.toContain('127.0.0.1');
  });
});
