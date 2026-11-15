// Run with: npm test
//
// A category label must open the right platform's category or nothing: a Kick
// id sent to Twitch's lookup, or a clip's placeholder name, opens a wrong page.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import {
  categoryFillOnly,
  categoryNavigation,
  categoryNeedsReload,
  categoryNote,
  categorySelectionKey,
  currentCategoryTarget,
  streamCategoryTarget,
} from './streamCategory.ts';

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

test('a selection is keyed by its id, or by its lowercased name when it has none', () => {
  assert.equal(categorySelectionKey({ id: '509658', name: 'Just Chatting' }), '509658');
  assert.equal(categorySelectionKey({ id: '', name: 'ASMR' }), 'asmr');
  assert.equal(categorySelectionKey({ id: '', name: '' }), '');
});

test('Home reloads only when a different category replaces a loaded one', () => {
  const a = { id: '1', name: 'A' };
  const b = { id: '2', name: 'B' };
  // A to B reloads, A to A does not.
  assert.equal(categoryNeedsReload('1', b), true);
  assert.equal(categoryNeedsReload('1', a), false);
  // Nothing loaded yet adopts the selection instead of reloading.
  assert.equal(categoryNeedsReload(null, a), false);
  assert.equal(categoryNeedsReload('', a), false);
  // A name-only selection reloads by name.
  assert.equal(categoryNeedsReload('1', { id: '', name: 'ASMR' }), true);
  assert.equal(categoryNeedsReload('asmr', { id: '', name: 'ASMR' }), false);
  // No selection never reloads.
  assert.equal(categoryNeedsReload('1', null), false);
});

test('the pass sequence: empty, adopt A, reload for B', () => {
  const a = { id: '1', name: 'A' };
  const b = { id: '2', name: 'B' };
  let loaded: string | null = null;
  const pass = (selected: { id: string; name: string }) => {
    const reload = categoryNeedsReload(loaded, selected);
    loaded = categorySelectionKey(selected);
    return reload;
  };
  assert.equal(pass(a), false);
  assert.equal(pass(a), false);
  assert.equal(pass(b), true);
});

test('a fill-only pass is the one that follows a name arriving for the same key', () => {
  const named = { id: '509658', name: 'Just Chatting' };
  assert.equal(categoryFillOnly({ key: '509658', name: '' }, named), true);
  // The name was already there.
  assert.equal(categoryFillOnly({ key: '509658', name: 'Just Chatting' }, named), false);
  // Another category.
  assert.equal(categoryFillOnly({ key: '1', name: '' }, named), false);
  // Nothing loaded, an empty key, no selection, or a name that has not arrived.
  assert.equal(categoryFillOnly(null, named), false);
  assert.equal(categoryFillOnly({ key: '', name: '' }, { id: '', name: 'x' }), false);
  assert.equal(categoryFillOnly({ key: '509658', name: '' }, null), false);
  assert.equal(categoryFillOnly({ key: '509658', name: '' }, { id: '509658', name: '' }), false);
});

test('a named category keeps the words Home shows today', () => {
  assert.equal(categoryNote('live', 'Rust'), 'No one is streaming Rust.');
  assert.equal(categoryNote('clips', 'Rust'), 'No clips have been generated for Rust yet.');
  assert.equal(categoryNote('videos', 'Rust'), 'No videos exist for Rust.');
  assert.equal(categoryNote('tags', 'Rust', 1), 'No live streams in Rust match that tag.');
  assert.equal(categoryNote('tags', 'Rust', 2), 'No live streams in Rust match those tags.');
  assert.equal(categoryNote('tags', 'Rust'), 'No live streams in Rust match those tags.');
});

test('a nameless category never reads the empty name', () => {
  assert.equal(categoryNote('live', ''), 'No one is live in this category right now.');
  assert.equal(categoryNote('tags', ''), 'No one is live in this category right now.');
  assert.equal(categoryNote('clips', ''), 'No clips have been generated for this category yet.');
  assert.equal(categoryNote('videos', ''), 'No videos exist for this category.');
  for (const kind of ['live', 'tags', 'clips', 'videos'] as const) {
    const text = categoryNote(kind, '');
    assert.ok(!text.includes('  '), text);
    assert.ok(!text.includes(' .'), text);
  }
});
