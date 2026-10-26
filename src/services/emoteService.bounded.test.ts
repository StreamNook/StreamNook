// Run with: npm test
//
// The window's emote id -> file map is capped, and an id that fell out of it
// resolves exactly as one never cached: no disk URL, and it can be asked for again.

import { test, vi } from 'vitest';
import assert from 'node:assert/strict';

const hoisted = vi.hoisted(() => ({
  arrived: null as null | ((files: Record<string, string>) => void),
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  convertFileSrc: (path: string) => `asset://localhost/${encodeURIComponent(path)}`,
}));
const requestAssetCaching = vi.fn();
vi.mock('./assetCacheQueue', () => ({
  onAssetsCached: (kind: string, handlers: { arrived: (files: Record<string, string>) => void }) => {
    if (kind === 'emote') hoisted.arrived = handlers.arrived;
  },
  requestAssetCaching: (...args: unknown[]) => requestAssetCaching(...args),
  setAssetCacheBurst: vi.fn(),
  forgetAssetRequests: vi.fn(),
}));

const { getCachedEmoteUrl, queueEmoteForCaching, enhanceRustEmotes, rememberRowEmoteFile } = await import('./emoteService');

const CAP = 5000;

test('the emote file map holds 5000 entries and drops the oldest', () => {
  const files: Record<string, string> = {};
  for (let i = 0; i <= CAP; i++) files[`twitch-e${i}`] = `C:\\cache\\e${i}.png`;
  hoisted.arrived!(files);

  assert.equal(getCachedEmoteUrl('e0', 'twitch', '2x'), undefined);
  assert.ok(getCachedEmoteUrl('e1', 'twitch', '2x'));
  assert.ok(getCachedEmoteUrl(`e${CAP}`, 'twitch', '2x'));
});

test('an evicted emote id is requested again like a never-cached one', () => {
  requestAssetCaching.mockClear();
  // Still held: no request.
  queueEmoteForCaching('twitch-e1', 'https://cdn/e1');
  assert.equal(requestAssetCaching.mock.calls.length, 0);
  // Evicted: goes through the normal request path.
  queueEmoteForCaching('twitch-e0', 'https://cdn/e0');
  assert.equal(requestAssetCaching.mock.calls.length, 1);
  assert.deepEqual(requestAssetCaching.mock.calls[0].slice(0, 3), ['emote', 'twitch-e0', 'https://cdn/e0']);
});

test('stamped set rows and history rows write through the same cap', () => {
  const rows = [];
  for (let i = 0; i <= CAP; i++) rows.push({ id: `s${i}`, name: `S${i}`, url: `https://cdn/s${i}`, provider: 'twitch', local_url: `C:\\cache\\s${i}.png` });
  enhanceRustEmotes(rows, '2x');
  assert.equal(getCachedEmoteUrl('s0', 'twitch', '2x'), undefined);
  assert.ok(getCachedEmoteUrl('s1', 'twitch', '2x'));
  assert.ok(getCachedEmoteUrl(`s${CAP}`, 'twitch', '2x'));

  rememberRowEmoteFile('rowfresh', 'C:\\cache\\rowfresh.webp');
  assert.ok(getCachedEmoteUrl('rowfresh', '7tv'));
  // The write that landed at the cap pushed the oldest one out.
  assert.equal(getCachedEmoteUrl('s1', 'twitch', '2x'), undefined);
});
