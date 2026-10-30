// The pill's words: one phrase per identification and state, kept for that
// capture, never the one picked just before, every phrase reachable, and the
// breathing dots only after words that are still going.
import { afterEach, describe, expect, it } from 'vitest';
import {
  FINDING_PHRASES,
  FOUND_LABELS,
  isOngoing,
  LISTENING_PHRASES,
  MISS_PHRASES,
  PHRASES,
  phraseFor,
  pickPhrase,
  setPhraseRandom,
  type PhraseKind,
} from './listeningWords';

afterEach(() => setPhraseRandom(null));

const KINDS: PhraseKind[] = ['listening', 'finding', 'found', 'miss'];

describe('the phrases', () => {
  it('are the four lists, none of them saying the obvious "the song"', () => {
    expect(PHRASES.listening).toBe(LISTENING_PHRASES);
    expect(PHRASES.finding).toBe(FINDING_PHRASES);
    expect(PHRASES.found).toBe(FOUND_LABELS);
    expect(PHRASES.miss).toBe(MISS_PHRASES);
    for (const kind of KINDS) {
      expect(PHRASES[kind].length).toBeGreaterThan(1);
      for (const p of PHRASES[kind]) expect(p).not.toMatch(/the song/i);
    }
  });

  it('carry the dots only where the words are still going', () => {
    for (const p of ['Lemme hear that', 'Vibing', 'Tuning in', 'Cooking', 'Digging through the crates']) {
      expect(isOngoing(p), p).toBe(true);
    }
    for (const p of ['Almost got it', 'Say less', 'Locked in', 'Putting a name to it']) expect(isOngoing(p), p).toBe(false);
  });
});

describe('the picker', () => {
  it('never picks the same phrase twice in a row', () => {
    for (const kind of KINDS) {
      let previous: string | null = null;
      for (let i = 0; i < 200; i++) {
        const next = pickPhrase(PHRASES[kind], previous);
        expect(next).not.toBe(previous);
        expect(PHRASES[kind]).toContain(next);
        previous = next;
      }
    }
  });

  it('reaches every phrase', () => {
    for (const kind of KINDS) {
      const list = PHRASES[kind];
      const seen = new Set<string>();
      // Every slot of the random range, after every possible previous pick.
      for (const previous of [null, ...list]) {
        for (let i = 0; i < list.length; i++) seen.add(pickPhrase(list, previous, () => (i + 0.5) / list.length));
      }
      expect([...seen].sort()).toEqual([...list].sort());
    }
  });

  it('keeps a capture’s words, and gives the next capture different ones', () => {
    const first = phraseFor('listening', 'capture-a');
    expect(phraseFor('listening', 'capture-a')).toBe(first);
    expect(phraseFor('listening', 'capture-b')).not.toBe(first);
    // Its finding words are picked apart from its listening words.
    expect(FINDING_PHRASES).toContain(phraseFor('finding', 'capture-a'));
  });
});
