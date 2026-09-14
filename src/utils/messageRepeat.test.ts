// Run with: npm test
//
// The normalizer decides what counts as "the same message", so a mistake here
// either folds messages that aren't the same or fails to fold a real wave.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import { normalizeForRepeat, isPrivilegedChatter } from './messageRepeat.ts';

test('folds case, spacing and trailing punctuation by default', () => {
  const key = normalizeForRepeat('LULW');
  assert.equal(normalizeForRepeat('lulw'), key);
  assert.equal(normalizeForRepeat('  LULW  '), key);
  assert.equal(normalizeForRepeat('LULW!!'), key);
  assert.equal(normalizeForRepeat('LULW...'), key);
});

test('collapses runs of internal whitespace', () => {
  assert.equal(normalizeForRepeat('KEKW    KEKW'), normalizeForRepeat('KEKW KEKW'));
});

test('exact mode keeps case and trailing punctuation apart', () => {
  assert.notEqual(normalizeForRepeat('LULW', 'exact'), normalizeForRepeat('lulw', 'exact'));
  assert.notEqual(normalizeForRepeat('LULW', 'exact'), normalizeForRepeat('LULW!!', 'exact'));
  // Exact still trims, so surrounding whitespace alone is not a difference.
  assert.equal(normalizeForRepeat('  LULW  ', 'exact'), normalizeForRepeat('LULW', 'exact'));
});

test('emote-only messages take part — that is the main case', () => {
  assert.ok(normalizeForRepeat('OMEGALUL'));
  assert.equal(normalizeForRepeat('OMEGALUL'), normalizeForRepeat('omegalul'));
});

test('different messages do not share a key', () => {
  assert.notEqual(normalizeForRepeat('hello there'), normalizeForRepeat('hello there!'.replace('there', 'world')));
  assert.notEqual(normalizeForRepeat('LULW'), normalizeForRepeat('KEKW'));
});

test('opts out of empty messages and slash commands', () => {
  assert.equal(normalizeForRepeat(''), null);
  assert.equal(normalizeForRepeat('   '), null);
  assert.equal(normalizeForRepeat('/ban someone'), null);
  // Punctuation-only collapses to empty under normalization rather than
  // becoming a key every "!!!" would join.
  assert.equal(normalizeForRepeat('!!!'), null);
  // ...but exact mode has no stripping step, so it stays a real key.
  assert.ok(normalizeForRepeat('!!!', 'exact'));
});

test('truncates very long messages instead of holding them whole', () => {
  const key = normalizeForRepeat('a'.repeat(500));
  assert.ok(key);
  assert.equal(key!.length, 200);
});

test('privileged chatters are detected from the badges array', () => {
  assert.equal(isPrivilegedChatter([{ name: 'broadcaster' }]), true);
  assert.equal(isPrivilegedChatter([{ name: 'moderator' }]), true);
  assert.equal(isPrivilegedChatter([{ name: 'vip' }]), true);
  assert.equal(isPrivilegedChatter([{ name: 'subscriber' }, { name: 'vip' }]), true);
  assert.equal(isPrivilegedChatter([{ name: 'subscriber' }, { name: 'founder' }]), false);
  assert.equal(isPrivilegedChatter([]), false);
  assert.equal(isPrivilegedChatter(undefined), false);
  assert.equal(isPrivilegedChatter(null), false);
});
