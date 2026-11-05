// Rust caches the member sheet and rebuilds it when told a member's profile
// changed. One write still goes straight from the webview to Supabase instead of
// through Rust: the hidden-sections fallback when the site API is unreachable. It
// tells Rust once it has landed; a failed write tells nobody. (Accolades are no
// longer written from here: Rust saves them and tells itself.)
import { beforeEach, describe, expect, it, vi } from 'vitest';

const state = vi.hoisted(() => ({
  upsertError: null as { message: string } | null,
  /** Every command the webview invoked, with its arguments. */
  invoked: [] as Array<[string, unknown]>,
  /** What `streamnook_api_post` does: 'throw' sends the write to the direct fallback. */
  api: 'throw' as 'throw' | 'ok',
}));

vi.mock('@supabase/supabase-js', () => ({
  createClient: () => ({ from: () => ({ upsert: async () => ({ error: state.upsertError }) }) }),
  RealtimeChannel: class {},
  SupabaseClient: class {},
}));
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (command: string, args?: unknown) => {
    state.invoked.push([command, args]);
    if (command === 'streamnook_api_post') {
      if (state.api === 'throw') throw new Error('offline');
      return { status: 200, ok: true, body: '' };
    }
    return undefined;
  }),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

vi.stubEnv('VITE_SUPABASE_URL', 'https://example.supabase.co');
vi.stubEnv('VITE_SUPABASE_ANON_KEY', 'anon');

const load = () => import('./supabaseService');
const told = () => state.invoked.filter(([command]) => command === 'profile_changed');

describe('profile_changed after a direct write', () => {
  beforeEach(() => {
    state.upsertError = null;
    state.invoked = [];
    state.api = 'throw';
  });

  it('is sent once for the hidden-sections fallback write', async () => {
    await (await load()).setHiddenSections('42', ['messages']);
    expect(told()).toEqual([['profile_changed', { userId: '42' }]]);
  });

  it('is not sent when the fallback write failed', async () => {
    state.upsertError = { message: 'denied' };
    await (await load()).setHiddenSections('42', ['messages']);
    expect(told()).toEqual([]);
  });

  it('is not sent by the hidden-sections write that went through the site API', async () => {
    state.api = 'ok';
    await (await load()).setHiddenSections('42', ['messages']);
    expect(told()).toEqual([]);
  });
});

describe('accolades are saved by Rust only', () => {
  it('the webview has no direct accolade writer or day claim left', async () => {
    const service = (await load()) as Record<string, unknown>;
    expect(service.grantAccolade).toBeUndefined();
    expect(service.claimLoginAccolades).toBeUndefined();
    expect(typeof service.getAccolades).toBe('function');
  });
});
