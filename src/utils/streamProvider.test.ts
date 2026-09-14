// Run with: npm test
//
// The "bare = twitch" convention is load-bearing: a Twitch row must keep
// producing a BARE login key, because favorites, resume snapshots and chat
// slices persisted before multi-platform browsing all store bare logins.

import { describe, expect, it, test } from 'vitest';
import assert from 'node:assert/strict';

import { followIdentifier, streamProvider, streamKey, streamThumbnail, buildProviderUrl } from './streamProvider.ts';

test('an untagged row reads as twitch', () => {
  assert.equal(streamProvider({}), 'twitch');
  assert.equal(streamProvider(null), 'twitch');
  assert.equal(streamProvider({ provider: 'kick' }), 'kick');
});

test('twitch rows key bare, provider rows key composite', () => {
  assert.equal(streamKey({ user_login: 'XQC' }), 'xqc');
  assert.equal(streamKey({ user_login: 'XQC', provider: 'twitch' }), 'xqc');
  assert.equal(streamKey({ user_login: 'XQC', provider: 'kick' }), 'kick:xqc');
});

test('thumbnails fill twitch templates and pass provider URLs through', () => {
  assert.equal(
    streamThumbnail({ thumbnail_url: 'https://cdn/{width}x{height}.jpg' }, 1280, 720),
    'https://cdn/1280x720.jpg',
  );
  assert.equal(
    streamThumbnail({ thumbnail_url: 'https://cdn/%{width}x%{height}.jpg' }, 640, 360),
    'https://cdn/640x360.jpg',
  );
  const literal = 'https://images.kick.com/thumb.webp';
  assert.equal(streamThumbnail({ thumbnail_url: literal }, 1280, 720), literal);
  assert.equal(streamThumbnail({ thumbnail_url: '' }, 1280, 720), '');
});

test('watch URLs match what the backend classifier accepts', () => {
  assert.equal(buildProviderUrl('kick', 'xqc'), 'https://kick.com/xqc');
  assert.equal(buildProviderUrl('twitch', 'xqc'), 'https://twitch.tv/xqc');
  assert.equal(buildProviderUrl('tiktok', 'pokimane'), 'https://www.tiktok.com/@pokimane/live');
  assert.equal(buildProviderUrl('youtube', '@LinusTechTips'), 'https://www.youtube.com/@LinusTechTips/live');
  assert.equal(
    buildProviderUrl('youtube', 'UCXuqSBlHAE6Xw-yeJA0Tunw'),
    'https://www.youtube.com/channel/UCXuqSBlHAE6Xw-yeJA0Tunw/live',
  );
  assert.equal(buildProviderUrl('youtube', 'jfKfPfyJRdk'), 'https://www.youtube.com/watch?v=jfKfPfyJRdk');
});

test('a channel cannot break out of the URL it is interpolated into', () => {
  // Slugs reach this from search results and from raw text the user typed.
  assert.equal(buildProviderUrl('kick', 'a b'), 'https://kick.com/a%20b');
  assert.equal(buildProviderUrl('kick', 'a/b'), 'https://kick.com/a%2Fb');
  assert.equal(buildProviderUrl('twitch', 'a?b'), 'https://twitch.tv/a%3Fb');
  assert.equal(
    buildProviderUrl('youtube', 'a&b'),
    'https://www.youtube.com/watch?v=a%26b',
  );
});

test('no provider is ever addressed at twitch.tv', () => {
  for (const p of ['kick', 'youtube', 'tiktok'] as const) {
    assert.ok(!buildProviderUrl(p, 'somebody').includes('twitch.tv'), `${p} leaked to twitch.tv`);
  }
});

describe('followIdentifier', () => {
  it('keys Kick follows by the slug, never the numeric user id', () => {
    expect(
      followIdentifier({ provider: 'kick', user_login: 'theburntpeanut', user_id: '1234567' } as any),
    ).toBe('theburntpeanut');
  });

  it('keys YouTube follows by the channel UC id, not the per-broadcast login', () => {
    expect(
      followIdentifier({ provider: 'youtube', user_login: 'AGr94tpNVkw', user_id: 'UCabc123' } as any),
    ).toBe('UCabc123');
  });

  it('keys Twitch follows by the login', () => {
    expect(
      followIdentifier({ provider: 'twitch', user_login: 'nickmercs', user_id: '15564828' } as any),
    ).toBe('nickmercs');
  });

  it('falls back to the login when YouTube carries no channel id', () => {
    expect(
      followIdentifier({ provider: 'youtube', user_login: 'somelogin', user_id: '' } as any),
    ).toBe('somelogin');
  });
});
