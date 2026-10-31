// Run with: npm test
//
// The key rule here is the whole feature. A favourite that is keyed by anything
// the platform's live check won't accept is a favourite that silently never
// shows up as live, which is indistinguishable from the channel being offline.
//
// The cases the Rust and iPhone twins must also meet live in
// providerTwins.fixtures.json (`favorite_id`), read by providerTwins.test.ts.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import { dedupeByFavoriteId, favoriteIdOf } from './favorites.ts';

test('twitch favourites key by the bare numeric user id', () => {
  // Every favourite that predates multi-platform browsing is stored this way,
  // and Helix answers `streams?user_id=`.
  assert.equal(favoriteIdOf({ user_id: '71092938', user_login: 'xqc' }), '71092938');
  assert.equal(
    favoriteIdOf({ provider: 'twitch', user_id: '71092938', user_login: 'xqc' }),
    '71092938',
  );
});

test('kick favourites key by the SLUG even though the row carries a numeric id', () => {
  // Kick browse rows carry `broadcaster_user_id`, but Kick's live check queries
  // `slug=`. Keying on the id produces a favourite that never resolves.
  assert.equal(
    favoriteIdOf({ provider: 'kick', user_id: '12345', user_login: 'XQC' }),
    'kick:xqc',
  );
});

test('youtube favourites key by the channel, never the video id', () => {
  // `user_login` on a browse row is the VIDEO id: it names one broadcast and
  // never comes back.
  assert.equal(
    favoriteIdOf({ provider: 'youtube', user_id: 'UCabcDEF123', user_login: 'AGr94tpNVkw' }),
    'youtube:UCabcDEF123',
  );
  // A handle addresses a channel too, so it is an acceptable fallback.
  assert.equal(
    favoriteIdOf({ provider: 'youtube', user_id: '', user_login: '@somechannel' }),
    'youtube:@somechannel',
  );
});

test('a youtube row with no channel identity refuses to produce a key', () => {
  // The caller resolves one rather than persisting a video id.
  assert.equal(
    favoriteIdOf({ provider: 'youtube', user_id: '', user_login: 'AGr94tpNVkw' }),
    null,
  );
});

test('tiktok favourites key by handle', () => {
  assert.equal(
    favoriteIdOf({ provider: 'tiktok', user_id: '999', user_login: '@Someone' }),
    'tiktok:@someone',
  );
});

test('a login written as its own provider key names the bare channel', () => {
  // A Kick room's key is `kick:slug`; it must name the same favorite as the slug.
  assert.equal(favoriteIdOf({ provider: 'kick', user_id: '', user_login: 'kick:XQC' }), 'kick:xqc');
  assert.equal(favoriteIdOf({ provider: 'youtube', user_id: '', user_login: 'youtube:@chan' }), 'youtube:@chan');
});

test('dedupe collapses a youtube browse row and its live-check row', () => {
  // The exact duplicate a streamKey dedupe would let through: same channel,
  // keyed by video id from browse and by UC id from the sweep.
  const browse = {
    id: 'v1',
    user_id: 'UCabcDEF123',
    user_login: 'AGr94tpNVkw',
    user_name: 'Chan',
    title: '',
    viewer_count: 5,
    game_name: '',
    thumbnail_url: '',
    started_at: '',
    provider: 'youtube' as const,
  };
  const swept = { ...browse, user_login: 'UCabcDEF123', viewer_count: 7 };
  const out = dedupeByFavoriteId([browse, swept]);
  assert.equal(out.length, 1);
  assert.equal(out[0].viewer_count, 5, 'keeps the first row seen');
});

test('dedupe keeps genuinely different channels', () => {
  const rows = [
    { user_id: '1', user_login: 'a', user_name: 'a' },
    { user_id: '2', user_login: 'b', user_name: 'b' },
    { provider: 'kick' as const, user_id: '1', user_login: 'a', user_name: 'a' },
  ] as never[];
  assert.equal(dedupeByFavoriteId(rows).length, 3);
});
