// Run with: npm test
//
// The one rule that keeps a merged multi-platform feed from moderating the
// wrong account. Twitch and Kick user ids are both numeric strings, so a Kick
// id handed to Helix as a Twitch id is accepted and bans a real, unrelated
// person — silently, with a toast naming the Kick chatter.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import { isHomeRow, rowProvider } from './chatAuthority.ts';
import type { BackendChatMessage } from '../services/twitchChat.ts';

const row = (provider: string, userId = '676'): BackendChatMessage =>
  ({ id: 'm1', user_id: userId, username: 'someone', provider } as unknown as BackendChatMessage);

test('a row from the watched platform is actionable', () => {
  assert.equal(isHomeRow(row('twitch'), 'twitch'), true);
  assert.equal(isHomeRow(row('kick'), 'kick'), true);
});

test('a row from any other platform is not', () => {
  // The exact shape of the wrong-account ban: watching Twitch, a Kick row whose
  // numeric user id would be sent to Helix as a Twitch user id.
  assert.equal(isHomeRow(row('kick', '71092938'), 'twitch'), false);
  assert.equal(isHomeRow(row('youtube'), 'twitch'), false);
  assert.equal(isHomeRow(row('twitch'), 'kick'), false);
});

test('a raw IRC string belongs to the channel being watched', () => {
  // Strings only reach a slice from the home channel's own paths: the Twitch
  // read connection, an optimistic own-send, or an injected system row. Every
  // companion platform delivers structured objects instead.
  assert.equal(rowProvider('@id=abc;user-id=1 :x!x@x PRIVMSG #c :hi', 'twitch'), 'twitch');
  assert.equal(rowProvider('@id=abc PRIVMSG #c :hi', 'kick'), 'kick');
  assert.equal(isHomeRow('@id=abc PRIVMSG #c :hi', 'kick'), true);
});

test('a structured row with no provider is read as Twitch', () => {
  // Backend messages predate the provider field; the wire default is twitch.
  const legacy = { id: 'm1', user_id: '5', username: 'a' } as unknown as BackendChatMessage;
  assert.equal(rowProvider(legacy, 'twitch'), 'twitch');
  assert.equal(isHomeRow(legacy, 'twitch'), true);
  // ...which correctly makes it foreign when something else is being watched.
  assert.equal(isHomeRow(legacy, 'kick'), false);
});
