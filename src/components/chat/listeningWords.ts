// What the listening pill and the answer say: a phrase picked per
// identification (stable for that capture or answer), never the one picked
// the time before. Pure apart from its small memory, so it is tested without
// a page.

export const LISTENING_PHRASES = [
  'Lemme hear that',
  'Ears up',
  'Locked in',
  'Vibing',
  'Catching the vibe',
  "Ooh, what's this",
  'Say less',
  'Tuning in',
  'Hold up, listening',
  'Picking up the melody',
] as const;

export const FINDING_PHRASES = [
  'Let me cook',
  'Cooking',
  'Digging through the crates',
  'Running it down',
  'Checking the receipts',
  'Hold that thought',
  'Almost got it',
  'Putting a name to it',
] as const;

/** Over a found song's name, in the small uppercase label. */
export const FOUND_LABELS = ['Found it', 'Got it', "That's the one", 'Here it is', 'Banger found'] as const;

export const MISS_PHRASES = ["Couldn't catch it", "That one's a mystery", 'No clue on this one', "Didn't catch that"] as const;

export type PhraseKind = 'listening' | 'finding' | 'found' | 'miss';

export const PHRASES: Record<PhraseKind, readonly string[]> = {
  listening: LISTENING_PHRASES,
  finding: FINDING_PHRASES,
  found: FOUND_LABELS,
  miss: MISS_PHRASES,
};

/** Phrases that read as still going, which the pill follows with its
 *  breathing dots; the others end the thought. */
const ONGOING = new Set<string>(['Lemme hear that', 'Vibing', 'Tuning in', 'Cooking', 'Digging through the crates']);
export const isOngoing = (phrase: string): boolean => ONGOING.has(phrase);

/** One pick from `list`, never `previous` (when there is another). */
export function pickPhrase(list: readonly string[], previous: string | null, random: () => number = Math.random): string {
  const from = list.length > 1 && previous !== null ? list.filter((p) => p !== previous) : list;
  return from[Math.min(from.length - 1, Math.floor(random() * from.length))];
}

let random: () => number = Math.random;
/** For the harness and tests: the randomness the picks use. */
export function setPhraseRandom(fn: (() => number) | null): void {
  random = fn ?? Math.random;
}

const KEPT = 64;
const chosen = new Map<string, string>();
const last: Partial<Record<PhraseKind, string>> = {};

/** The phrase for this identification (or answer) `id`: picked once, then the
 *  same every time it is asked, so a capture keeps its words. */
export function phraseFor(kind: PhraseKind, id: string | number): string {
  const key = `${kind}:${id}`;
  const kept = chosen.get(key);
  if (kept) return kept;
  const phrase = pickPhrase(PHRASES[kind], last[kind] ?? null, random);
  last[kind] = phrase;
  chosen.set(key, phrase);
  if (chosen.size > KEPT) chosen.delete(chosen.keys().next().value as string);
  return phrase;
}
