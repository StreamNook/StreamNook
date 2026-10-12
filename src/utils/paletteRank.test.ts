// Run with: npm test
//
// The Ctrl+K palette ranks a search: an exact channel name or login must be
// the top row, so Enter opens that channel instead of a Quick Action whose
// words happen to contain the letters.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import { EXACT_MATCH_SCORE, orderSections, scoreItem } from './paletteRank';
import type { PaletteItem, PaletteSection } from './commandPaletteSources';

const item = (over: Partial<PaletteItem>): PaletteItem => ({
  id: 'x',
  section: 'Quick Actions',
  title: 'x',
  run: () => {},
  ...over,
});

const BASE: PaletteSection[] = ['Quick Actions', 'Settings', 'Followed Channels', 'Recent Chatters', 'Streamers', 'Snippets'];

test('a channel typed by its exact name or login scores as an exact match', () => {
  const streamer = item({ section: 'Streamers', title: 'RobCDee', keywords: 'robcdee robcdee', exactNames: ['robcdee', 'RobCDee'] });
  assert.equal(scoreItem(streamer, 'robcdee'), EXACT_MATCH_SCORE);
  // A display name the login does not spell still matches by login.
  const named = item({ section: 'Streamers', title: 'ロブ', exactNames: ['robcdee', 'ロブ'] });
  assert.equal(scoreItem(named, 'robcdee'), EXACT_MATCH_SCORE);
  // The letters scattered through a Quick Action's words are only a fuzzy hit.
  const home = item({ title: 'Go to Home', subtitle: 'Back to browse', keywords: 'home browse discover recommended categories' });
  const fuzzy = scoreItem(home, 'robcdee');
  assert.ok(fuzzy < EXACT_MATCH_SCORE);
  // A login that merely starts the same way is not exact.
  assert.ok(scoreItem(item({ title: 'robcdee_fan', exactNames: ['robcdee_fan'] }), 'robcdee') < EXACT_MATCH_SCORE);
});

test('the section holding the exact match renders first, channels before chatters', () => {
  const items = [
    { section: 'Quick Actions' as const, score: 90 },
    { section: 'Recent Chatters' as const, score: EXACT_MATCH_SCORE },
    { section: 'Streamers' as const, score: EXACT_MATCH_SCORE },
    { section: 'Settings' as const, score: 40 },
  ];
  const order = orderSections(BASE, items, ['Followed Channels', 'Streamers']);
  assert.deepEqual(order, ['Streamers', 'Recent Chatters', 'Quick Actions', 'Settings', 'Followed Channels', 'Snippets']);
});

test('without an exact match, and with no query, the base order stands', () => {
  assert.deepEqual(orderSections(BASE, [{ section: 'Streamers', score: 700 }]), BASE);
  assert.deepEqual(orderSections(BASE, [{ section: 'Streamers' }]), BASE);
  // A snippet alias prefix is a strong hit but not an exact one.
  assert.equal(scoreItem(item({ section: 'Snippets', title: 'Hype', alias: 'hyp' }), 'hy'), 1199);
  assert.deepEqual(orderSections(BASE, [{ section: 'Snippets', score: 1199 }]), BASE);
});
