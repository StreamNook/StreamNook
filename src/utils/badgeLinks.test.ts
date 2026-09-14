// Run with: npm test
//
// The GTA Drop badge rendered its category card twice: the campaign supplied one
// spelling and the prose parser another, the pre-resolve dedupe compared the
// names as written and saw two different strings, and both then resolved to the
// same Twitch id. The two cards were visibly separate, each fetching its own
// viewer count ("48,291+ watching" above "48,293+ watching").

import { test } from 'vitest';
import assert from 'node:assert/strict';

import { dedupeResolvedLinks } from './badgeLinks.ts';

test('two spellings that resolved to one category render once', () => {
  const links = [
    { type: 'category', name: 'Grand Theft Auto V', categoryId: '32982' },
    { type: 'category', name: 'Grand Theft Auto V', categoryId: '32982' },
  ];
  assert.deepEqual(dedupeResolvedLinks(links), [links[0]]);
});

test('the first link wins, so the authoritative one survives', () => {
  const links = [
    { type: 'category', name: 'Grand Theft Auto V', categoryId: '32982', authoritative: true },
    { type: 'category', name: 'Grand Theft Auto V', categoryId: '32982' },
  ];
  assert.equal(dedupeResolvedLinks(links)[0].authoritative, true);
});

test('different categories both survive', () => {
  const links = [
    { type: 'category', name: 'Grand Theft Auto V', categoryId: '32982' },
    { type: 'category', name: 'Just Chatting', categoryId: '509658' },
  ];
  assert.equal(dedupeResolvedLinks(links).length, 2);
});

test('a category and a drops card are different things', () => {
  const links = [
    { type: 'category', name: 'Marvel Rivals' },
    { type: 'drops', name: 'Marvel Rivals' },
  ];
  assert.equal(dedupeResolvedLinks(links).length, 2);
});

test('unresolved links fall back to the name, case-insensitively', () => {
  const links = [
    { type: 'category', name: 'Grand Theft Auto V' },
    { type: 'category', name: 'grand theft auto v' },
  ];
  assert.deepEqual(dedupeResolvedLinks(links), [links[0]]);
});

// An id is identity; a name is only a spelling. Twitch renaming a category
// mid-campaign must not produce two cards for one place.
test('the id decides, not the name', () => {
  const links = [
    { type: 'category', name: 'ELDEN RING NIGHTREIGN', categoryId: '55453844' },
    { type: 'category', name: 'Elden Ring: Nightreign', categoryId: '55453844' },
  ];
  assert.equal(dedupeResolvedLinks(links).length, 1);
});
