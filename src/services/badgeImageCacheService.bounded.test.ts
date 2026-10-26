// Run with: npm test
//
// The window's badge id -> file map is capped, and an id that fell out of it
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
    if (kind === 'badge') hoisted.arrived = handlers.arrived;
  },
  requestAssetCaching: (...args: unknown[]) => requestAssetCaching(...args),
}));

const { getCachedBadgeUrl, queueBadgeForCaching } = await import('./badgeImageCacheService');

const CAP = 5000;

test('the badge file map holds 5000 entries and drops the oldest', () => {
  const files: Record<string, string> = {};
  for (let i = 0; i <= CAP; i++) files[`b${i}`] = `C:\\cache\\b${i}.png`;
  hoisted.arrived!(files);

  assert.equal(getCachedBadgeUrl('b0'), undefined);
  assert.ok(getCachedBadgeUrl('b1'));
  assert.ok(getCachedBadgeUrl(`b${CAP}`));
});

test('an evicted badge id is requested again like a never-cached one', () => {
  requestAssetCaching.mockClear();
  queueBadgeForCaching('b1', 'https://cdn/b1');
  assert.equal(requestAssetCaching.mock.calls.length, 0);
  queueBadgeForCaching('b0', 'https://cdn/b0');
  assert.equal(requestAssetCaching.mock.calls.length, 1);
  assert.deepEqual(requestAssetCaching.mock.calls[0], ['badge', 'b0', 'https://cdn/b0']);
});
