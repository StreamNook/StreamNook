// Run with: npm test
//
// The key rule here is the whole feature. A favourite that is keyed by anything
// the platform's live check won't accept is a favourite that silently never
// shows up as live, which is indistinguishable from the channel being offline.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import { dedupeByFavoriteId, favoriteIdOf, favoriteMetaOf, isStrayYouTubeFavoriteId } from './favorites.ts';

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

test('the sidecar records the CHANNEL, not the broadcast that was on screen', () => {
  const meta = favoriteMetaOf(
    {
      id: 'v1',
      user_id: 'UCabcDEF123',
      user_login: 'AGr94tpNVkw',
      user_name: 'Some Channel',
      title: '',
      viewer_count: 0,
      game_name: '',
      thumbnail_url: '',
      started_at: '',
      provider: 'youtube',
      profile_image_url: 'https://example.invalid/a.png',
    },
    'youtube:UCabcDEF123',
  );
  assert.equal(meta.channel, 'UCabcDEF123');
  assert.equal(meta.provider, 'youtube');
  assert.equal(meta.display_name, 'Some Channel');
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

test('the stray-id repair catches a bare UC id and nothing else', () => {
  // Real data: the old sidebar wrote stream.user_id raw, so a YouTube favorite
  // landed as a bare UC id that parseKey reads back as a Twitch login.
  assert.equal(isStrayYouTubeFavoriteId('UCMNEVbszv8ZyvSXoTn3yhpQ'), true);
  // Must never touch a Twitch id (always numeric) or an already-keyed entry.
  assert.equal(isStrayYouTubeFavoriteId('180118013'), false);
  assert.equal(isStrayYouTubeFavoriteId('youtube:UCMNEVbszv8ZyvSXoTn3yhpQ'), false);
  // A Twitch login that happens to start with UC is not 24 characters of id.
  assert.equal(isStrayYouTubeFavoriteId('ucantseeme'), false);
});
