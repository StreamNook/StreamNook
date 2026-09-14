// Run with: npm test
//
// These rules decide what the spell checker is even allowed to look at. Get
// them wrong in one direction and every emote in the composer turns red; get
// them wrong in the other and real typos slip through unflagged.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import {
  tokenizeForSpellcheck,
  getSpellcheckTarget,
  adjacentTranspositions,
} from './chatInputWord.ts';

const words = (text: string): string[] =>
  tokenizeForSpellcheck(text).map((t) => t.word);

test('keeps ordinary words, including contractions', () => {
  assert.deepEqual(words('i recieve teh msgs'), ['recieve', 'teh', 'msgs']);
  assert.deepEqual(words("dont you mean don't"), ['dont', 'you', 'mean', "don't"]);
});

test('skips Twitch vocabulary', () => {
  // Mentions, channels, commands, cheers, emoji shortcodes.
  assert.deepEqual(words('@brandon /ban !uptime $tip :smile:'), []);
  // Emote names: camelCase and all-caps.
  assert.deepEqual(words('pepeLaugh PogChamp monkaS KEKW OMEGALUL'), []);
  // Logins carry digits or underscores.
  assert.deepEqual(words('xqc_ Kappa123 user_name'), []);
  // Links and timestamps.
  assert.deepEqual(words('twitch.tv/xqc 12:30 https://example.com'), []);
  // Too short to be worth flagging.
  assert.deepEqual(words('ok gg ez o7'), []);
});

test('strips surrounding punctuation from the range', () => {
  // The comma stays out of the range, so replacing the word keeps it.
  assert.deepEqual(tokenizeForSpellcheck('i recieve, ok'), [
    { word: 'recieve', start: 2, end: 9 },
  ]);
  assert.deepEqual(tokenizeForSpellcheck('(teh)'), [
    { word: 'teh', start: 1, end: 4 },
  ]);
});

test('finds the word under the caret', () => {
  assert.deepEqual(getSpellcheckTarget('i recieve, ok', 5, 5), {
    word: 'recieve',
    start: 2,
    end: 9,
  });
});

test('does not span a newline', () => {
  // The composer accepts Shift+Enter, so a space-only split would return 0..11.
  assert.deepEqual(getSpellcheckTarget('one\nrecieve', 6, 6), {
    word: 'recieve',
    start: 4,
    end: 11,
  });
});

test('returns null on a word the checker should ignore', () => {
  assert.equal(getSpellcheckTarget('nice PogChamp', 8, 8), null);
  assert.equal(getSpellcheckTarget('hey @brandon', 9, 9), null);
});

test('a single-word selection wins over the caret', () => {
  // Right-clicking inside a selection leaves the selection intact, so the
  // highlighted word is what the user means.
  assert.deepEqual(getSpellcheckTarget('i recieve teh', 10, 13), {
    word: 'teh',
    start: 10,
    end: 13,
  });
  // A selection spanning several words has no single target.
  assert.equal(getSpellcheckTarget('i recieve teh', 2, 13), null);
});

test('generates every adjacent transposition', () => {
  // The dictionary never proposes these itself, so "the" only reaches the menu
  // if this produces it.
  assert.deepEqual(adjacentTranspositions('teh'), ['eth', 'the']);
  assert.deepEqual(adjacentTranspositions('adn'), ['dan', 'and']);
  // Case rides along, so a capitalised typo yields a capitalised fix.
  assert.ok(adjacentTranspositions('Teh').includes('The'));
  // Nothing to swap.
  assert.deepEqual(adjacentTranspositions('a'), []);
  assert.deepEqual(adjacentTranspositions(''), []);
});
