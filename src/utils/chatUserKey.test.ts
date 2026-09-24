// Run with: npm test
//
// The chat-user store indexes chatters by name so a mention can render that
// person's colour and cosmetics. Keyed by the bare name, two platforms' chatters
// of the same name overwrite each other on every message, and whoever spoke last
// decides whose cosmetics a mention paints — on a real person's row, silently.
//
// The ids themselves were always namespaced; only the name index was not, which
// is why this survived: the store looked correct at every call site.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import { usernameKey } from './chatterIdentity.ts';

test('the same name on two platforms indexes under two keys', () => {
  const twitchBob = usernameKey('12345', 'bob');
  const kickBob = usernameKey('kick:676', 'bob');
  assert.notEqual(twitchBob, kickBob);
  assert.equal(twitchBob, 'bob');
  assert.equal(kickBob, 'kick:bob');
});

test('Twitch keeps the bare name, matching the rest of the app', () => {
  assert.equal(usernameKey('12345', 'Bob'), 'bob');
  assert.equal(usernameKey('12345', 'BOB'), 'bob', 'and folds case');
});

test('the platform is read off the id rather than tracked separately', () => {
  // One source of truth: the id is already namespaced by whoever wrote it, so
  // the name index cannot disagree with it about which platform this is.
  assert.equal(usernameKey('youtube:UCabc', 'bob'), 'youtube:bob');
  assert.equal(usernameKey('tiktok:99', 'bob'), 'tiktok:bob');
});

test('a name containing a colon does not confuse the derivation', () => {
  // The split is on the ID, never on the name, so a name can hold anything.
  assert.equal(usernameKey('12345', 'we:rd'), 'we:rd');
  assert.equal(usernameKey('kick:676', 'we:rd'), 'kick:we:rd');
});
