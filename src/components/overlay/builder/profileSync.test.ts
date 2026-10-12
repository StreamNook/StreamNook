// Run with: npm test
//
// The overlay list is the account's once overlays sync between the app and the
// site. These pin the rule: server copies win, overlays gone from the account
// drop out, drafts are left alone.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import {
  overlayNeedsSave,
  overlaySaveSnapshot,
  profileFromServer,
  profileSaveSnapshot,
  reconcileProfiles,
  type OverlayProfile,
} from './profileSync';
import { DEFAULT_OVERLAY_STYLE } from '../overlayConfig';

const local = (uid: string, name: string, id: string | null, fontSize = 15, channel = 'winters27'): OverlayProfile => ({
  uid,
  name,
  id,
  version: id ? 'v-old' : null,
  style: { ...DEFAULT_OVERLAY_STYLE, fontSize },
  sources: channel ? [{ provider: 'twitch', channel }] : [],
});
const row = (id: string, name: string, fontSize: number, updated = 'v-new') => ({
  id,
  channels: [{ provider: 'twitch', channel: 'winters27' }],
  style: { profileName: name, fontSize },
  updated_at: updated,
});

test('a published overlay takes the server copy and keeps its uid', () => {
  const { profiles } = reconcileProfiles([local('u1', 'Main', 'A', 15)], [row('A', 'Main renamed', 22)]);
  assert.equal(profiles.length, 1);
  assert.equal(profiles[0].uid, 'u1');
  assert.equal(profiles[0].name, 'Main renamed');
  assert.equal(profiles[0].style.fontSize, 22);
  assert.equal(profiles[0].version, 'v-new');
});

test('an overlay deleted elsewhere drops out and is reported', () => {
  const { profiles, dropped } = reconcileProfiles([local('u1', 'Main', 'A'), local('u2', 'Old', 'B')], [row('A', 'Main', 15)]);
  assert.deepEqual(profiles.map((p) => p.id), ['A']);
  assert.deepEqual(dropped, ['Old']);
});

test('drafts are left exactly as they are', () => {
  const draft = local('u9', 'Draft', null, 30);
  const { profiles } = reconcileProfiles([local('u1', 'Main', 'A'), draft], [row('A', 'Main', 15)]);
  assert.equal(profiles[1], draft);
});

test('account overlays the list does not have yet are added after the local ones', () => {
  const { profiles } = reconcileProfiles([local('u1', 'Main', 'A')], [row('A', 'Main', 15), row('C', 'Made on the site', 18)]);
  assert.deepEqual(profiles.map((p) => p.name), ['Main', 'Made on the site']);
  assert.equal(profiles[1].id, 'C');
});

test('the blank starting overlay gives way to the account overlays', () => {
  const blank = local('u0', 'Default', null, 15, '');
  const { profiles } = reconcileProfiles([blank], [row('A', 'Main', 15)]);
  assert.deepEqual(profiles.map((p) => p.id), ['A']);
});

test('a draft with channels is not the blank starting overlay', () => {
  const draft = local('u0', 'Default', null, 15, 'winters27');
  const { profiles } = reconcileProfiles([draft], [row('A', 'Main', 15)]);
  assert.deepEqual(profiles.map((p) => p.id), [null, 'A']);
});

test('the list is never empty', () => {
  const { profiles } = reconcileProfiles([local('u1', 'Main', 'A')], []);
  assert.equal(profiles.length, 1);
  assert.equal(profiles[0].id, null);
});

test('two local copies claiming one overlay collapse to one', () => {
  const { profiles } = reconcileProfiles([local('u1', 'Main', 'A'), local('u2', 'Main copy', 'A')], [row('A', 'Main', 15)]);
  assert.deepEqual(profiles.map((p) => p.uid), ['u1']);
});

// Auto-save sends only a real edit. A one-shot "skip the load" flag is not
// enough: StrictMode runs the effect twice on mount, the second run saved the
// overlay as loaded, and the only thing that wrote was a default (gifScale: 1)
// the server copy lacked. Comparing snapshots holds however often it runs.
test('an overlay as loaded gives the same save snapshot however its look is spelled', () => {
  // The server copy predates gifScale; the editor holds it filled out.
  const server = profileFromServer(
    { id: 'A', channels: [{ provider: 'twitch', channel: 'winters27' }], style: { profileName: 'Main', fontSize: 18 }, updated_at: 'v1' },
    'Main',
    'u1',
  ) as OverlayProfile;
  const stored = { profileName: 'Main', fontSize: 18 } as Partial<OverlayProfile['style']>;
  assert.equal(server.style.gifScale, 1);
  assert.equal(overlaySaveSnapshot(server.style, server.sources), overlaySaveSnapshot(stored, server.sources));
  // Key order is not content.
  const reordered = Object.fromEntries(Object.entries(server.style).reverse()) as OverlayProfile['style'];
  assert.equal(overlaySaveSnapshot(reordered, server.sources), overlaySaveSnapshot(server.style, server.sources));
  // Nothing beyond provider and channel rides on a source.
  const extra = [{ provider: 'twitch' as const, channel: 'winters27', note: 'x' }];
  assert.equal(overlaySaveSnapshot(server.style, extra), overlaySaveSnapshot(server.style, server.sources));
});

test('a real edit to the look or the sources changes the save snapshot', () => {
  const p = local('u1', 'Main', 'A', 15);
  const saved = overlaySaveSnapshot(p.style, p.sources);
  assert.notEqual(overlaySaveSnapshot({ ...p.style, fontSize: 16 }, p.sources), saved);
  assert.notEqual(overlaySaveSnapshot({ ...p.style, gifScale: 1.5 }, p.sources), saved);
  assert.notEqual(overlaySaveSnapshot(p.style, [...p.sources, { provider: 'kick', channel: 'winters27' }]), saved);
  assert.notEqual(overlaySaveSnapshot(p.style, []), saved);
  // Put back, it is the saved copy again: nothing to send.
  assert.equal(overlaySaveSnapshot({ ...p.style, fontSize: 15 }, p.sources), saved);
  // Setting a value to its default IS an edit when the saved copy held another.
  const big = overlaySaveSnapshot({ ...p.style, gifScale: 2 }, p.sources);
  assert.notEqual(overlaySaveSnapshot({ ...p.style, gifScale: 1 }, p.sources), big);
});

// A rename used to nudge the style without changing it, and the snapshot held
// only the look and sources, so a published overlay's new name never reached
// the account. The name is part of what a save sends, so it is part of the
// comparison.
test('renaming a published overlay is a real difference; renaming it back is not', () => {
  const p = local('u1', 'Main', 'A', 15);
  const saved = profileSaveSnapshot(p);
  assert.equal(overlayNeedsSave(p, saved), false);
  assert.equal(overlayNeedsSave({ ...p, name: 'Main cam' }, saved), true);
  // Undone (or typed back) before the debounce: the saved copy again.
  assert.equal(overlayNeedsSave({ ...p, name: 'Main' }, saved), false);
  // Look and sources still count.
  assert.equal(overlayNeedsSave({ ...p, style: { ...p.style, fontSize: 16 } }, saved), true);
  assert.equal(overlayNeedsSave({ ...p, sources: [] }, saved), true);
});

test('a draft never auto-saves, renamed or edited', () => {
  const d = local('u2', 'Draft', null, 15);
  const saved = profileSaveSnapshot(d);
  assert.equal(overlayNeedsSave({ ...d, name: 'Draft renamed' }, saved), false);
  assert.equal(overlayNeedsSave({ ...d, style: { ...d.style, fontSize: 30 } }, saved), false);
});

test('an overlay as loaded from the account needs no save', () => {
  const server = profileFromServer(row('A', 'Main', 18), 'Main', 'u1') as OverlayProfile;
  assert.equal(overlayNeedsSave(server, profileSaveSnapshot(server)), false);
  assert.equal(overlayNeedsSave({ ...server, name: 'Renamed elsewhere' }, profileSaveSnapshot(server)), true);
});
