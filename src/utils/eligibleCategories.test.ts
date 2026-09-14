// Run with: npm test
//
// The input is the real Pichu badge copy. Its category list is the reason this
// code exists: three of its entries contain commas or ampersands, and two of
// the pieces "Animals, Aquariums, and Zoos" splits into are themselves real
// Twitch categories (verified live, 2026-09-04), so any reading that resolves
// short pieces first gets it confidently wrong.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import {
  categoryLookupNames,
  categoryRuns,
  eligibleCategoryList,
  readCategoryList,
} from './eligibleCategories.ts';

const PICHU = [
  'By watching a participating channel for 20 minutes on 3 different days, you can earn a free Poké Ball.',
  '',
  'Full list of eligible categories:',
  'Pokémon FireRed/LeafGreen, Pokémon Pokopia, Pokémon UNITE, Pokémon GO, Pokémon Legends: Z-A,',
  'Pokémon Scarlet/Violet, Just Chatting, DJs, Art, Special Events, Sports, Music,',
  'Talk Shows & Podcasts, Animals, Aquariums, and Zoos, Co-working & Studying, and Makers & Crafting',
  'Evolution: Twitch has announced that evolved versions of these badges are coming.',
  'Official description of the new campaign: https://help.twitch.tv/s/article/pokemon-chat-badges',
].join('\n');

// What Twitch actually answers for these names, measured 2026-09-04.
const REAL = new Set(
  [
    'Pokémon FireRed/LeafGreen', 'Pokémon Pokopia', 'Pokémon UNITE', 'Pokémon GO',
    'Pokémon Legends: Z-A', 'Pokémon Scarlet/Violet', 'Just Chatting', 'DJs', 'Art',
    'Special Events', 'Sports', 'Music', 'Talk Shows & Podcasts',
    'Animals, Aquariums, and Zoos', 'Co-working & Studying', 'Makers & Crafting',
    // Real on their own too, which is the whole trap:
    'Animals', 'Aquariums',
  ].map((n) => n.toLowerCase())
);
const exists = (name: string) => REAL.has(name.toLowerCase());

test('the list stops before the labelled lines that follow it', () => {
  const list = eligibleCategoryList(PICHU);
  assert.ok(list);
  assert.ok(list.startsWith('Pokémon FireRed/LeafGreen'));
  assert.ok(list.endsWith('Makers & Crafting'));
  assert.ok(!list.includes('Evolution:'));
  assert.ok(!list.includes('help.twitch.tv'));
});

test('no list means no categories, not a guess', () => {
  assert.equal(eligibleCategoryList('Subscribe to /studbudz during the event.'), null);
  assert.deepEqual(categoryRuns('Subscribe to /studbudz during the event.'), []);
});

test('reads every category the copy names, in order', () => {
  assert.deepEqual(readCategoryList(PICHU, exists), [
    'Pokémon FireRed/LeafGreen',
    'Pokémon Pokopia',
    'Pokémon UNITE',
    'Pokémon GO',
    'Pokémon Legends: Z-A',
    'Pokémon Scarlet/Violet',
    'Just Chatting',
    'DJs',
    'Art',
    'Special Events',
    'Sports',
    'Music',
    'Talk Shows & Podcasts',
    'Animals, Aquariums, and Zoos',
    'Co-working & Studying',
    'Makers & Crafting',
  ]);
});

// The specific failure this ordering prevents.
test('a name containing commas beats the real categories it starts with', () => {
  const got = readCategoryList(PICHU, exists);
  assert.ok(got.includes('Animals, Aquariums, and Zoos'));
  assert.ok(!got.includes('Animals'), 'the standalone Animals category must not win');
  assert.ok(!got.includes('Aquariums'), 'the standalone Aquariums category must not win');
});

test('the trailing "and" of the last item is not part of its name', () => {
  const names = categoryLookupNames(PICHU);
  assert.ok(names.includes('Makers & Crafting'));
  assert.ok(!names.includes('and Makers & Crafting'));
});

test('a name Twitch does not know is dropped rather than rendered', () => {
  const text = 'Full list of eligible categories:\nJust Chatting, Not A Real Category, Art';
  assert.deepEqual(readCategoryList(text, exists), ['Just Chatting', 'Art']);
});

test('every candidate is asked about once, longest first', () => {
  const text = 'Full list of eligible categories:\nAnimals, Aquariums, and Zoos';
  const runs = categoryRuns(text);
  assert.deepEqual(runs[0].map((r) => r.pieces), [3, 2, 1]);
  assert.equal(runs[0][0].name, 'Animals, Aquariums, and Zoos');
  const names = categoryLookupNames(text);
  assert.equal(new Set(names.map((n) => n.toLowerCase())).size, names.length);
});
