// Run with: npm test
//
// Rust stamps each cached emote's disk path on the set it hands a page, so no
// window has to pull the whole disk-cache index (tens of thousands of entries)
// to render emotes from disk. These pin the page's half of that contract.

import { test, vi, beforeEach } from 'vitest';
import assert from 'node:assert/strict';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
  convertFileSrc: (path: string) => `asset://localhost/${encodeURIComponent(path)}`,
}));
vi.mock('./assetCacheQueue', () => ({
  onAssetsCached: vi.fn(),
  requestAssetCaching: vi.fn(),
  setAssetCacheBurst: vi.fn(),
  forgetAssetRequests: vi.fn(),
}));

const { enhanceRustEmotes, fetchAllEmotes, getCachedEmoteUrl } = await import('./emoteService');

beforeEach(() => invoke.mockReset());

test('a stamped path becomes the row localUrl and feeds the per-message lookup', () => {
  const [kappa, cdnOnly] = enhanceRustEmotes(
    [
      { id: '25', name: 'Kappa', url: 'https://cdn/25', provider: 'twitch', local_url: 'C:\\cache\\twitch-25.png' },
      { id: '88', name: 'PogChamp', url: 'https://cdn/88', provider: 'twitch' },
    ],
    '2x',
  );
  assert.equal(kappa.localUrl, 'asset://localhost/C%3A%5Ccache%5Ctwitch-25.png');
  // The raw path never travels further than this function.
  assert.equal('local_url' in kappa, false);
  assert.equal(cdnOnly.localUrl, undefined);
  // Chat rows look emotes up by id; the stamped one now resolves from disk.
  assert.equal(getCachedEmoteUrl('25', 'twitch', '2x'), kappa.localUrl);
  assert.equal(getCachedEmoteUrl('88', 'twitch', '2x'), undefined);
});

test('7TV paths are remembered at the tier the set was fetched for', () => {
  enhanceRustEmotes([{ id: 'abc', name: 'catJAM', url: 'https://7tv/abc', provider: '7tv', local_url: '/c/abc@1x.avif' }], '1x');
  assert.ok(getCachedEmoteUrl('abc', '7tv', '1x'));
  assert.equal(getCachedEmoteUrl('abc', '7tv', '4x'), undefined);
});

test('loading a channel set is one call, with no index load and no page-side token', async () => {
  invoke.mockImplementation(async (cmd: string) =>
    cmd === 'fetch_channel_emotes' ? { twitch: [], bttv: [], '7tv': [], ffz: [], kick: [], seven_tv_ok: true } : undefined,
  );
  await fetchAllEmotes('xqc', '71092938');
  const calls = invoke.mock.calls.filter((c) => typeof c[0] === 'string');
  const commands = calls.map((c) => c[0]);
  assert.deepEqual(commands.filter((c) => c === 'fetch_channel_emotes').length, 1);
  assert.equal(commands.includes('get_cached_files'), false);
  assert.equal(commands.includes('get_twitch_token'), false);
  const args = calls.find((c) => c[0] === 'fetch_channel_emotes')![1] as Record<string, unknown>;
  assert.equal('accessToken' in args, false);
  assert.ok(['1x', '2x', '3x', '4x'].includes(args.tier as string));
});
