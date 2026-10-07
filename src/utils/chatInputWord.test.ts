// Run with: npm test
//
// These rules decide what the spell checker is even allowed to look at. Get
// them wrong in one direction and every emote in the composer turns red; get
// them wrong in the other and real typos slip through unflagged.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import {
  insertAtCaret,
  emoteSearchTrigger,
  withChatterCandidates,
  wrapIndex,
  TAB_CYCLE_LIMIT,
  tokenizeForSpellcheck,
  getSpellcheckTarget,
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

// --- Emote list trigger ------------------------------------------------------

test('a colon opening a word, then two characters, opens the emote list', () => {
  assert.deepEqual(emoteSearchTrigger(':lo', 3), { anchor: 0, query: 'lo' });
  assert.deepEqual(emoteSearchTrigger('hi :lo', 6), { anchor: 3, query: 'lo' });
  assert.deepEqual(emoteSearchTrigger('hi\n:Pog', 7), { anchor: 3, query: 'Pog' });
  // Only what is before the caret counts.
  assert.deepEqual(emoteSearchTrigger(':lo world', 3), { anchor: 0, query: 'lo' });
});

test('times, emoticons, links and short queries never open it', () => {
  assert.equal(emoteSearchTrigger('12:30', 5), null);
  assert.equal(emoteSearchTrigger(':)', 2), null);
  assert.equal(emoteSearchTrigger(':))', 3), null);
  assert.equal(emoteSearchTrigger(':D', 2), null);
  assert.equal(emoteSearchTrigger('https://x.tv', 12), null);
  assert.equal(emoteSearchTrigger('::lo', 4), null);
  assert.equal(emoteSearchTrigger(':l', 2), null);
  assert.equal(emoteSearchTrigger('hi :lo', 2), null);
});

// --- Tab cycle ---------------------------------------------------------------

test('wrapIndex wraps both ways', () => {
  assert.equal(wrapIndex(0, 1, 3), 1);
  assert.equal(wrapIndex(2, 1, 3), 0);
  assert.equal(wrapIndex(0, -1, 3), 2);
  assert.equal(wrapIndex(0, 7, 3), 1);
  assert.equal(wrapIndex(0, 1, 0), 0);
});

test('chatters follow the emotes and skip names an emote already has', () => {
  const emotes = [{ name: 'lol', priority: 0 }];
  const users = [
    { username: 'lol', displayName: 'LOL' },
    { username: 'lorenzo', displayName: 'Lorenzo' },
    { username: 'bob', displayName: 'Bob' },
  ];
  const out = withChatterCandidates(emotes, 'lo', users, 'starts_with');
  assert.deepEqual(out.map((c) => c.name), ['lol', 'Lorenzo']);
  assert.equal(out[1].chatter?.username, 'lorenzo');
});

test('an @ query completes to @name and a colon query is emote-only', () => {
  const users = [{ username: 'lorenzo', displayName: 'Lorenzo' }];
  assert.deepEqual(withChatterCandidates([], '@lo', users, 'starts_with').map((c) => c.name), ['@Lorenzo']);
  assert.equal(withChatterCandidates([], ':lo', users, 'starts_with').length, 0);
});

test('the cycle never grows past its limit', () => {
  const emotes = Array.from({ length: TAB_CYCLE_LIMIT + 5 }, (_, i) => ({ name: `e${i}`, priority: i }));
  const out = withChatterCandidates(emotes, 'e', [{ username: 'eve' }], 'starts_with');
  assert.equal(out.length, TAB_CYCLE_LIMIT);
});

// The emote menu (in the chat or popped out into its own window) puts its
// pick where the caret was left, not at the end of the message.
test('insertAtCaret spaces the pick from its neighbours', () => {
  assert.deepEqual(insertAtCaret('', 0, 0, 'Kappa'), { text: 'Kappa ', caret: 6 });
  assert.deepEqual(insertAtCaret('hi', 2, 2, 'Kappa'), { text: 'hi Kappa ', caret: 9 });
  assert.deepEqual(insertAtCaret('hi ', 3, 3, 'Kappa'), { text: 'hi Kappa ', caret: 9 });
  assert.deepEqual(insertAtCaret('there', 0, 0, 'Kappa'), { text: 'Kappa there', caret: 6 });
  assert.deepEqual(insertAtCaret('a  b', 2, 2, 'Kappa'), { text: 'a Kappa b', caret: 7 });
});

test('insertAtCaret replaces a selection and clamps stale positions', () => {
  assert.deepEqual(insertAtCaret('one two three', 4, 7, 'LUL'), { text: 'one LUL three', caret: 7 });
  // A caret past the end (the text shrank since it was read) appends.
  assert.deepEqual(insertAtCaret('hi', 99, 99, 'Kappa'), { text: 'hi Kappa ', caret: 9 });
  // An end before the start never cuts text.
  assert.deepEqual(insertAtCaret('hi', 2, 0, 'Kappa'), { text: 'hi Kappa ', caret: 9 });
  // Emoji are inserted whole.
  assert.deepEqual(insertAtCaret('gg', 2, 2, '\u{1F525}'), { text: 'gg \u{1F525} ', caret: 6 });
});
