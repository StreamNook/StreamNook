// Run with: npm test
//
// Raw platform user ids overlap: Kick user 676 and Twitch user 676 are different
// people. Every key built from one has to say which platform it came from, or
// the two share a bucket and each shows the other's message history and 7TV
// cosmetics under a real person's name.
//
// The failure that makes this worth a test: the chat panel derives this key
// while walking the message array, and used to take the platform from a
// PANEL-level value meaning "the channel being watched". That is correct while
// the panel shows one channel and wrong the moment it shows a merged feed, and
// nothing about it fails loudly.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import { historyKey, chatterProvider, chatterId } from './chatterIdentity.ts';
import { rowProvider } from './chatAuthority.ts';
import type { BackendChatMessage } from '../services/twitchChat.ts';

test('the same raw id on two platforms produces two different keys', () => {
  assert.notEqual(historyKey('676', 'kick'), historyKey('676', 'twitch'));
  assert.equal(historyKey('676', 'kick'), 'kick:676');
});

test('Twitch stays bare, so history stored before multi-platform still resolves', () => {
  assert.equal(historyKey('676', 'twitch'), '676');
  assert.equal(historyKey('676'), '676', 'and when the platform is not given at all');
});

test('a merged feed keys each row by its OWN platform, not the watched one', () => {
  // Watching Twitch, with the streamer's Kick chat blended in.
  const home = 'twitch';
  const twitchRow = { id: 'a', user_id: '676', provider: 'twitch' } as unknown as BackendChatMessage;
  const kickRow = { id: 'b', user_id: '676', provider: 'kick' } as unknown as BackendChatMessage;

  const keyOf = (m: BackendChatMessage) => historyKey(m.user_id, rowProvider(m, home));

  assert.equal(keyOf(twitchRow), '676');
  assert.equal(keyOf(kickRow), 'kick:676');
  assert.notEqual(
    keyOf(twitchRow),
    keyOf(kickRow),
    'taking the platform from the panel would have collapsed these into one bucket',
  );
});

test('a chatter id comes from the tag on Twitch and the struct elsewhere', () => {
  // Kick and YouTube carry no IRC user-id tag; reading only the tag is why
  // clicking one of their chatters used to do nothing at all.
  assert.equal(chatterId({ provider: 'twitch', tags: new Map([['user-id', '5']]) }), '5');
  assert.equal(chatterId({ provider: 'kick', tags: new Map(), providerUserId: '676' }), '676');
  assert.equal(chatterId({ provider: 'kick', tags: new Map([['user-id', '5']]) }), undefined);
  assert.equal(chatterProvider({ tags: new Map() }), 'twitch', 'a bare row is Twitch');
});
