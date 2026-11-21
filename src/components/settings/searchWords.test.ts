// Run with: npm test
//
// Settings search reads loose words: filler dropped, phrases kept whole, other
// words for the same idea accepted, and a long query needs only most of its
// ideas. Found when "readability", "emote names", "name next to the emote" and
// "emote name beside" all came back empty although Emote words was what each
// person meant. Rust's phone search runs the same cases (settings_catalog.rs).

import { describe, expect, it } from 'vitest';
import cases from './searchWords.cases.json';
import words from './searchWords.json';
import { searchSettings } from './searchIndex';
import { conceptsRequired, fieldHasWord, queryConcepts, searchField, searchStem } from './searchWords';

describe('the shared word list', () => {
  it('puts each word in one group only', () => {
    const all = words.groups.flat();
    expect(all.length).toBe(new Set(all).size);
  });

  it('keeps filler out of the groups', () => {
    const filler = new Set(words.filler);
    expect(words.groups.flat().filter((w) => filler.has(w))).toEqual([]);
  });
});

describe('reading a query', () => {
  it.each(cases.concepts.map((c) => [c.query, c]))('"%s"', (_q, c) => {
    const concepts = queryConcepts(c.query);
    expect(concepts.map((k) => k.literal)).toEqual(c.literals);
    for (const w of c.alike) {
      expect(concepts.some((k) => k.alike.includes(w)), `no concept accepts "${w}"`).toBe(true);
    }
  });

  it('needs every idea of a short query and most of a long one', () => {
    expect([1, 2, 3, 4, 5, 6].map(conceptsRequired)).toEqual([1, 2, 2, 3, 3, 4]);
  });

  it('takes plural and verb endings off a word', () => {
    expect(['names', 'labels', 'muted', 'loading', 'glass', 'ads'].map(searchStem)).toEqual([
      'name',
      'label',
      'mut',
      'load',
      'glass',
      'ads',
    ]);
  });

  it('matches whole words with an ending, never inside another word', () => {
    const f = searchField('Show the word typed with each emote, next to it. Loading ads.');
    expect(fieldHasWord(f, 'emote')).toBe(true);
    expect(fieldHasWord(f, 'next to')).toBe(true);
    expect(fieldHasWord(f, 'ad')).toBe(true);
    expect(fieldHasWord(f, 'load')).toBe(true);
    expect(fieldHasWord(f, 'mote')).toBe(false);
    expect(fieldHasWord(f, 'ype')).toBe(false);
  });
});

describe('the desktop settings search', () => {
  it.each(cases.finds.map((c) => [c.query, c]))('finds the row meant by "%s"', (_q, c) => {
    const titles = searchSettings(c.query).map((e) => e.title);
    expect(titles.slice(0, c.top), `got ${titles.slice(0, 8).join(', ')}`).toContain(c.title);
  });

  it('still ranks an exact title first', () => {
    expect(searchSettings('emote words')[0]?.title).toBe('Emote words');
    expect(searchSettings('word size')[0]?.title).toBe('Word size');
  });

  it('puts rows that say the typed word itself above rows that only say a word like it', () => {
    // Emote words says "emote captions"; Word size says "emote" and "caption size".
    const titles = searchSettings('emote captions').map((e) => e.title);
    expect(titles).toContain('Word size');
    expect(titles.indexOf('Emote words')).toBeLessThan(titles.indexOf('Word size'));
  });

  it('still finds nothing for words no row means', () => {
    expect(searchSettings('zzzz qqqq')).toEqual([]);
  });
});
