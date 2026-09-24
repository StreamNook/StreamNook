import { beforeEach, describe, expect, it, vi } from 'vitest';

import { reloadSnippetStore, storedOf, useSnippetStore, viewOf } from './snippetStore';

const invokeMock = vi.hoisted(() => vi.fn());

// Node's own localStorage stub lacks most of the Storage API here.
const memory = new Map<string, string>();
vi.stubGlobal('localStorage', {
  getItem: (k: string) => memory.get(k) ?? null,
  setItem: (k: string, v: string) => void memory.set(k, String(v)),
  removeItem: (k: string) => void memory.delete(k),
  clear: () => memory.clear(),
});

vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}), emit: vi.fn() }));

const stored = {
  custom: [{ id: 'custom.gg.ab12', title: 'gg', category: 'Hype', content: 'GG', keywords: 'win' }],
  favorites: ['custom.gg.ab12', 'classic.kappa'],
  aliases: { 'custom.gg.ab12': 'gg' },
};

function settingsWith(snippets: unknown) {
  return async (cmd: string) => (cmd === 'load_settings' ? { snippets } : undefined);
}

describe('snippetStore', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    localStorage.clear();
  });

  it('round-trips the stored section through the view', () => {
    const view = viewOf(stored);
    expect(view.customSnippets[0]).toMatchObject({ id: 'custom.gg.ab12', custom: true, keywords: 'win' });
    expect(view.favoriteIds.has('classic.kappa')).toBe(true);
    expect(view.aliases.get('custom.gg.ab12')).toBe('gg');
    expect(storedOf(view)).toEqual(stored);
  });

  it('reads what it can from a damaged section', () => {
    const view = viewOf({ custom: [{ id: 1 }, stored.custom[0]] as never, favorites: [3, 'a'] as never, aliases: { x: ' AB ', y: '' } });
    expect(view.customSnippets).toHaveLength(1);
    expect([...view.favoriteIds]).toEqual(['a']);
    expect([...view.aliases]).toEqual([['x', 'ab']]);
  });

  it('moves old localStorage snippets into settings once, then drops them', async () => {
    localStorage.setItem('streamnook.snippets.custom.v1', JSON.stringify(stored.custom));
    localStorage.setItem('streamnook.snippets.favorites.v1', JSON.stringify(stored.favorites));
    invokeMock.mockImplementation(settingsWith(undefined));

    await reloadSnippetStore();

    const patch = invokeMock.mock.calls.find((c) => c[0] === 'patch_settings');
    expect(patch?.[1]).toMatchObject({ patch: { snippets: { custom: stored.custom, favorites: stored.favorites } } });
    expect(localStorage.getItem('streamnook.snippets.custom.v1')).toBeNull();
    expect(useSnippetStore.getState().customSnippets).toHaveLength(1);
  });

  it('never overwrites snippets settings already hold', async () => {
    localStorage.setItem('streamnook.snippets.favorites.v1', JSON.stringify(['old']));
    invokeMock.mockImplementation(settingsWith(stored));

    await reloadSnippetStore();

    expect(invokeMock.mock.calls.some((c) => c[0] === 'patch_settings')).toBe(false);
    expect(localStorage.getItem('streamnook.snippets.favorites.v1')).toBeNull();
    expect(useSnippetStore.getState().favoriteIds.has('old')).toBe(false);
  });

  it('a change is shown at once and saved as the snippets key alone', () => {
    invokeMock.mockResolvedValue(undefined);
    useSnippetStore.setState(viewOf(stored));
    useSnippetStore.getState().toggleFavorite('classic.kappa');

    expect(useSnippetStore.getState().favoriteIds.has('classic.kappa')).toBe(false);
    const [cmd, args] = invokeMock.mock.calls.at(-1)!;
    expect(cmd).toBe('patch_settings');
    expect(Object.keys(args.patch)).toEqual(['snippets']);
    expect(args.patch.snippets.favorites).toEqual(['custom.gg.ab12']);
  });
});
