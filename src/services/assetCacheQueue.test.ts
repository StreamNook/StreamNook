import { beforeEach, describe, expect, it, vi } from 'vitest';

import { forgetAssetRequests, onAssetsCached, requestAssetCaching } from './assetCacheQueue';

const invokeMock = vi.hoisted(() => vi.fn(async (..._args: unknown[]) => undefined));
const handlers = new Map<string, (event: { payload: unknown }) => void>();
const deliver = (payload: unknown) => handlers.get('asset-cache://cached')?.({ payload });

vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    handlers.set(name, handler);
    return () => {};
  }),
}));
vi.mock('../utils/platform', () => ({ IS_MOBILE: false }));

const settle = () => new Promise((r) => setTimeout(r, 0));

describe('assetCacheQueue', () => {
  beforeEach(() => {
    invokeMock.mockClear();
    forgetAssetRequests('emote');
    forgetAssetRequests('badge');
  });

  it('sends one call per kind and priority for a tick of requests, dropping repeats', async () => {
    requestAssetCaching('emote', 'a', 'https://cdn/a');
    requestAssetCaching('emote', 'a', 'https://cdn/a');
    requestAssetCaching('emote', 'b', 'https://cdn/b');
    requestAssetCaching('emote', 's', 'https://cdn/s', true);
    requestAssetCaching('badge', 'x', 'https://cdn/x');
    await settle();

    const calls = invokeMock.mock.calls.map((c) => c[1]);
    expect(invokeMock.mock.calls.every((c) => c[0] === 'asset_cache_enqueue')).toBe(true);
    expect(calls).toEqual([
      { kind: 'emote', priority: false, items: [{ id: 'a', url: 'https://cdn/a' }, { id: 'b', url: 'https://cdn/b' }] },
      { kind: 'emote', priority: true, items: [{ id: 's', url: 'https://cdn/s' }] },
      { kind: 'badge', priority: false, items: [{ id: 'x', url: 'https://cdn/x' }] },
    ]);

    requestAssetCaching('emote', 'a', 'https://cdn/a');
    await settle();
    expect(invokeMock).toHaveBeenCalledTimes(3);
  });

  it('passes arrivals to the kind that asked, and lets that id be asked again', async () => {
    const emotes: Record<string, string> = {};
    onAssetsCached('emote', { arrived: (files) => Object.assign(emotes, files), cleared: () => {} });
    requestAssetCaching('emote', 'c', 'https://cdn/c');
    await settle();

    deliver({ kind: 'emote', files: { c: 'C:/cache/c.webp' } });
    deliver({ kind: 'badge', files: { z: 'C:/cache/z.png' } });
    expect(emotes).toEqual({ c: 'C:/cache/c.webp' });

    invokeMock.mockClear();
    requestAssetCaching('emote', 'c', 'https://cdn/c');
    await settle();
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it('a wipe in any window empties every map and allows asking again', async () => {
    let cleared = 0;
    onAssetsCached('badge', { arrived: () => {}, cleared: () => { cleared += 1; } });
    requestAssetCaching('badge', 'w', 'https://cdn/w');
    await settle();
    handlers.get('asset-cache://cleared')?.({ payload: null });
    expect(cleared).toBe(1);

    invokeMock.mockClear();
    requestAssetCaching('badge', 'w', 'https://cdn/w');
    await settle();
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it('ignores requests without an id or url', async () => {
    requestAssetCaching('emote', '', 'https://cdn/a');
    requestAssetCaching('emote', 'q', '');
    await settle();
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
