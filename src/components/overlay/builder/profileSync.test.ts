// Run with: npm test
//
// The overlay list is the account's once overlays sync between the app and the
// site. These pin the rule: server copies win, overlays gone from the account
// drop out, drafts are left alone.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import { reconcileProfiles, type OverlayProfile } from './profileSync';
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
