// English spell checking, off the main thread.
//
// Parsing the dictionary takes a few hundred milliseconds and holds a sizeable
// word map afterwards, so it lives in a worker: the composer never janks while
// it loads, and closing the window frees it.
//
// This worker knows nothing about Twitch. Emote names, chatter logins and the
// user's own dictionary are filtered out on the main thread before words get
// here (see utils/spellcheck.ts), which keeps those rules unit-testable and
// keeps the worker from needing store access.
//
// Nothing leaves the device: the dictionary is bundled at build time and there
// is no network call anywhere in this file.

import nspell from 'nspell';
import { adjacentTranspositions } from '../utils/chatInputWord';
import aff from './dictionaries/en_US.aff?raw';
import dic from './dictionaries/en_US.dic?raw';

/** Past this length, `suggest` spends a long time generating candidates that
 *  are never right anyway. Keyboard-mash gets no suggestions, instantly. */
const MAX_SUGGEST_LENGTH = 20;

/** Ranked suggestions are long; the menu only has room for a handful. */
const MAX_SUGGESTIONS = 5;

export type SpellRequest =
  | { id: number; type: 'check'; words: string[] }
  | { id: number; type: 'suggest'; word: string }
  | { id: number; type: 'warm' };

export type SpellResponse =
  | { id: number; type: 'check'; misspelled: string[] }
  | { id: number; type: 'suggest'; correct: boolean; suggestions: string[] }
  | { id: number; type: 'warm' }
  | { id: number; type: 'error'; message: string };

let speller: ReturnType<typeof nspell> | null = null;

/** Built once, on the first request of any kind. Every handler awaits this so
 *  concurrent messages during startup queue behind a single parse. */
function ensureSpeller(): ReturnType<typeof nspell> {
  if (!speller) speller = nspell({ aff, dic });
  return speller;
}

self.onmessage = (event: MessageEvent<SpellRequest>) => {
  const request = event.data;

  try {
    const spell = ensureSpeller();

    switch (request.type) {
      case 'warm':
        post({ id: request.id, type: 'warm' });
        break;

      case 'check':
        post({
          id: request.id,
          type: 'check',
          misspelled: request.words.filter((word) => !spell.correct(word)),
        });
        break;

      case 'suggest':
        post({
          id: request.id,
          type: 'suggest',
          correct: spell.correct(request.word),
          suggestions:
            request.word.length > MAX_SUGGEST_LENGTH ? [] : suggestFor(spell, request.word),
        });
        break;
    }
  } catch (err) {
    post({
      id: request.id,
      type: 'error',
      message: err instanceof Error ? err.message : String(err),
    });
  }
};

/**
 * Corrections for one word, best first.
 *
 * Transpositions go ahead of the dictionary's own ranking. Swapping two letters
 * you typed in the wrong order is both the most common typo and the most
 * confident fix — if "the" is a candidate for "teh", it is almost certainly the
 * word that was meant, and burying it under "ten, eh, meh" is the wrong answer.
 * `adjacentTranspositions` explains why these have to be generated here rather
 * than coming out of the dictionary.
 */
function suggestFor(spell: ReturnType<typeof nspell>, word: string): string[] {
  // A correctly spelled word has nothing to correct. This guard is not
  // redundant: plenty of real words sit one transposition away from another
  // real word — form/from, angel/angle, quite/quiet, dairy/diary — so without
  // it, right-clicking a perfectly good word offers to "fix" it. nspell's own
  // suggest() early-returns for correct words; the transposition pass does not.
  if (spell.correct(word)) return [];

  const swapped = adjacentTranspositions(word).filter((candidate) => spell.correct(candidate));

  const seen = new Set<string>();
  const ranked: string[] = [];

  for (const suggestion of [...swapped, ...spell.suggest(word)]) {
    const key = suggestion.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    ranked.push(suggestion);
    if (ranked.length === MAX_SUGGESTIONS) break;
  }

  return ranked;
}

function post(response: SpellResponse): void {
  self.postMessage(response);
}
