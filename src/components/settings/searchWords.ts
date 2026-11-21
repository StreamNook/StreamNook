// How a settings search reads what someone typed: the webview twin of
// `query_concepts` in Rust's `services/settings_catalog.rs`, which the phones'
// search runs. It lives in TS because the desktop index (SETTINGS_INDEX in
// searchIndex.ts) is a TS table searched on every keystroke, and a round trip
// per key would slow the box down. Rust and this file read the same words
// (searchWords.json) and run the same cases (searchWords.cases.json).
//
// A query becomes a list of ideas ("concepts"): filler words are dropped, a
// phrase from a group ("next to", "hard to read") is one idea, and each idea
// carries the other words that mean the same ("beside", "under", "after"), so
// "emote name beside" finds Emote words although none of its rows says
// "beside". A longer query only needs most of its ideas to land, so one word
// the rows never use no longer empties the list.
import words from './searchWords.json';

export interface SearchConcept {
  /** What was typed for this idea: one word, or a phrase from a group. */
  literal: string;
  /** Other words that mean the same here, matched as whole words. */
  alike: string[];
}

/** Lowercase, anything but letters, digits, `&`, `!` and `@` read as a space. */
export const normalizeSearchText = (s: string): string =>
  s.toLowerCase().replace(/[^\p{L}\p{N}&!@]+/gu, ' ').trim();

const FILLER = new Set<string>(words.filler);
const GROUP_OF = new Map<string, string[]>();
for (const group of words.groups) for (const w of group) GROUP_OF.set(w, group);
const MAX_PHRASE = Math.max(...[...words.groups.flat(), ...words.filler].map((w) => w.split(' ').length));

/** Endings a whole-word match accepts, so "label" finds "labels" and "labeled". */
const SUFFIXES = ['', 's', 'es', 'e', 'd', 'ed', 'er', 'ing'];

/** A typed word with a plural or verb ending taken off ("names" -> "name"). */
export const searchStem = (w: string): string => {
  if (w.length > 4 && w.endsWith('ing')) return w.slice(0, -3);
  if (w.length > 4 && w.endsWith('ed')) return w.slice(0, -2);
  if (w.length > 3 && w.endsWith('s') && !w.endsWith('ss')) return w.slice(0, -1);
  return w;
};

const groupFor = (key: string, stem: string): string[] | undefined =>
  GROUP_OF.get(key) ?? GROUP_OF.get(stem) ?? GROUP_OF.get(`${stem}e`) ?? GROUP_OF.get(`${key}s`);

/** The ideas in a query, in the order typed, each once. */
export const queryConcepts = (query: string): SearchConcept[] => {
  const raw = query.toLowerCase().split(/\s+/).filter(Boolean);
  const keys = raw.map(normalizeSearchText);
  const concepts: SearchConcept[] = [];
  const seen = new Set<string>();
  const add = (literal: string, id: string, alike: string[]) => {
    if (seen.has(id)) return;
    seen.add(id);
    concepts.push({ literal, alike: [...new Set(alike)].filter((w) => w !== literal) });
  };
  let i = 0;
  outer: while (i < raw.length) {
    for (let n = Math.min(MAX_PHRASE, raw.length - i); n >= 2; n--) {
      const phrase = keys.slice(i, i + n).join(' ');
      if (FILLER.has(phrase)) {
        i += n;
        continue outer;
      }
      const group = GROUP_OF.get(phrase);
      if (group) {
        add(phrase, group[0], group);
        i += n;
        continue outer;
      }
    }
    const literal = raw[i];
    const key = keys[i];
    i++;
    if (key && FILLER.has(key)) continue;
    const stem = searchStem(key);
    const group = key ? groupFor(key, stem) : undefined;
    const alike = [...(group ?? []), ...(stem !== key && stem.length >= 3 ? [stem] : [])].filter((w) => w !== key);
    add(literal, group?.[0] ?? (stem || literal), alike);
  }
  // Nothing but filler ("on the"): search it as typed.
  if (concepts.length === 0) return raw.map((literal) => ({ literal, alike: [] }));
  return concepts;
};

/** How many of a query's ideas a row must answer: all of one or two, most of more. */
export const conceptsRequired = (count: number): number => (count <= 2 ? count : Math.ceil(count * 0.6));

/** One searchable text of a row, read once: its lowercase text and its words. */
export interface SearchField {
  raw: string;
  padded: string;
  words: Set<string>;
}

export const searchField = (text: string | undefined): SearchField => {
  const norm = normalizeSearchText(text ?? '');
  return { raw: (text ?? '').toLowerCase(), padded: ` ${norm.replace(/\s+/g, ' ')} `, words: new Set(norm.split(/\s+/).filter(Boolean)) };
};

/** Whether the field holds `word` (or phrase) as a whole word, with an ending allowed. */
export const fieldHasWord = (field: SearchField, word: string): boolean =>
  word.includes(' ')
    ? SUFFIXES.some((s) => field.padded.includes(` ${word}${s} `))
    : SUFFIXES.some((s) => field.words.has(word + s));

/** Whether the field holds any word that means the same as the concept. */
export const fieldHasAlike = (field: SearchField, concept: SearchConcept): boolean =>
  concept.alike.some((w) => fieldHasWord(field, w));
