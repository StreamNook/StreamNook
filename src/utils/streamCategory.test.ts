// Run with: npm test
//
// A category label must open the right platform's category or nothing: a Kick
// id sent to Twitch's lookup, or a clip's placeholder name, opens a wrong page.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import { categoryNavigation, currentCategoryTarget, streamCategoryTarget } from './streamCategory.ts';

test('twitch opens by id when it has one', () => {
  assert.deepEqual(streamCategoryTarget({ game_id: '509658', game_name: 'Just Chatting' }), {
    kind: 'twitch',
    category: { id: '509658', name: 'Just Chatting', box_art_url: '' },
  });
});

test('twitch without an id still opens, by name', () => {
  assert.equal(streamCategoryTarget({ game_name: 'ASMR' })?.category.id, '');
});

test('kick opens its own category only with an id', () => {
  const t = streamCategoryTarget({ provider: 'kick', game_id: '15', game_name: 'Just Chatting' });
  assert.equal(t?.kind, 'provider');
  assert.equal(streamCategoryTarget({ provider: 'kick', game_name: 'Just Chatting' }), null);
});

test('no category, or a platform without categories, opens nothing', () => {
  assert.equal(streamCategoryTarget({ game_name: '  ' }), null);
  assert.equal(streamCategoryTarget(null), null);
  assert.equal(streamCategoryTarget({ provider: 'tiktok', game_id: '1', game_name: 'Chat' }), null);
});

test('the player links live categories only', () => {
  assert.equal(currentCategoryTarget({ game_name: 'Twitch Clip' }, 'clip'), null);
  assert.equal(currentCategoryTarget({ game_name: 'Twitch Video' }, 'video'), null);
  assert.equal(currentCategoryTarget({ game_id: '9', game_name: 'ASMR' }, 'offline_chat')?.category.id, '');
  assert.equal(currentCategoryTarget({ game_id: '9', game_name: 'ASMR' }, 'live')?.category.id, '9');
  assert.equal(currentCategoryTarget(null, 'live'), null);
});

test('a twitch category opens on Live with no forward step', () => {
  const target = streamCategoryTarget({ game_id: '9', game_name: 'ASMR' })!;
  assert.deepEqual(categoryNavigation(target, 'all'), {
    platform: null,
    patch: {
      homeActiveTab: 'category',
      homeSelectedCategory: { id: '9', name: 'ASMR', box_art_url: '' },
      homeLastExitedCategory: null,
      homeCategoryTab: 'live',
    },
  });
  assert.equal(categoryNavigation(target, 'twitch').platform, null);
  assert.equal(categoryNavigation(target, 'kick').platform, 'twitch');
});

test("a kick category opens on Kick's Categories tab", () => {
  const target = streamCategoryTarget({ provider: 'kick', game_id: '15', game_name: 'Just Chatting' })!;
  assert.equal(categoryNavigation(target, 'all').platform, 'kick');
  assert.equal(categoryNavigation(target, 'twitch').platform, 'kick');
  assert.deepEqual(categoryNavigation(target, 'kick'), {
    platform: null,
    patch: {
      homeActiveTab: 'browse',
      homeProviderCategory: {
        provider: 'kick',
        id: '15',
        name: 'Just Chatting',
        thumbnail: '',
        viewer_count: 0,
        channel_count: 0,
      },
      homeLastExitedProviderCategory: null,
    },
  });
});
