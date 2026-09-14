/**
 * Find the start/end indices of the word at the cursor position in a string.
 * A word is a contiguous run of non-space characters.
 *
 * Examples (cursor marked with |):
 *   "hello| world"        -> [0, 5]   ("hello")
 *   "hello |world"        -> [6, 11]  ("world")
 *   "Kappa| HeyGuys"      -> [0, 5]   ("Kappa")
 *   "abc def gh|ij klm"   -> [8, 12]  ("ghij")
 */
export function getWordRange(text: string, position: number): [number, number] {
  let start = 0;
  let end = text.length;

  for (let i = position; i >= 0; i--) {
    if (i === 0 || text.charAt(i - 1) === ' ') {
      start = i;
      break;
    }
  }

  for (let i = position; i <= text.length; i++) {
    if (i === text.length || text.charAt(i) === ' ') {
      end = i;
      break;
    }
  }

  return [start, end];
}

export type EmoteTabMatchMode = 'starts_with' | 'includes';

export interface EmoteTabCandidate {
  name: string;
  priority: number;
  emote?: {
    id: string;
    name: string;
    url: string;
    localUrl?: string;
    provider: 'twitch' | 'bttv' | '7tv' | 'ffz' | 'kick' | 'youtube';
    isZeroWidth?: boolean;
    /** FFZ modifier bitmask; present only on FFZ modifier emotes */
    modifierFlags?: number;
    /** FFZ effect composable only by FFZ subscribers */
    ffzSubOnly?: boolean;
  };
  /** Set for chatter completions; the value is prefixed with @ if user typed @-prefix */
  chatter?: { username: string; displayName: string };
}

// --- Spell-check tokenizing ------------------------------------------------
//
// Twitch chat is mostly words the dictionary has never heard of: emotes,
// logins, commands, cheers. Flagging those would bury the one real typo in a
// wall of red, so a token has to earn its way INTO the checker rather than out
// of it. Everything here is pure text work — the emote set, the chatter list
// and the user's own dictionary are consulted separately (see utils/spellcheck).

/** One candidate word plus where it sits in the source text. */
export interface SpellToken {
  word: string;
  start: number;
  end: number;
}

/** Characters that can start or end a word we still want to check. Trimming to
 *  letters keeps "recieve," and "(teh)" checkable without swallowing the
 *  punctuation, so replacing the word leaves the punctuation alone. */
const isLetter = (c: string): boolean => /\p{L}/u.test(c);

/** Decide whether a raw whitespace-delimited chunk is worth spell checking.
 *  `raw` is the untrimmed chunk (leading @ and / still attached); `word` is the
 *  letter-trimmed core. */
function isCheckable(raw: string, word: string): boolean {
  // Too short to be a meaningful typo, and short words are where the false
  // positives cluster (gg, ez, o7).
  if (word.length < 3) return false;

  // Mentions, channels, slash commands, bot commands, emoji shortcodes, cheers.
  if (/^[@#/!:$]/.test(raw)) return false;

  // Logins and emote variants almost always carry a digit or an underscore.
  // Tested against the RAW chunk: trimming has already pulled the trailing "_"
  // off "xqc_" and the digits off "Kappa123", so checking the trimmed core here
  // would let both straight through.
  if (/[\d_]/.test(raw)) return false;

  // Links, timestamps, ratios. Checked on the raw chunk so "twitch.tv/x" is
  // caught even though trimming would leave letters at both ends.
  if (/[./:]/.test(raw)) return false;

  // An uppercase letter after the first character means camelCase, which is how
  // nearly every emote name is written: pepeLaugh, PogChamp, HeyGuys, monkaS.
  if (/\p{Lu}/u.test(word.slice(1))) return false;

  // Fully uppercase is KEKW / OMEGALUL / LULW territory. This does mean an
  // all-caps typo goes unflagged; in Twitch chat that trade is worth it.
  if (word === word.toUpperCase()) return false;

  return true;
}

/**
 * Split text into the words worth spell checking, with their offsets.
 *
 * Splits on ALL whitespace (not just spaces) because the composer accepts
 * newlines via Shift+Enter.
 *
 *   "i recieve, Kappa"  ->  [{ word: 'recieve', start: 2, end: 9 }]
 */
export function tokenizeForSpellcheck(text: string): SpellToken[] {
  const tokens: SpellToken[] = [];
  const chunk = /\S+/g;
  let match: RegExpExecArray | null;

  while ((match = chunk.exec(text)) !== null) {
    const raw = match[0];
    let start = match.index;
    let end = match.index + raw.length;

    // Trim non-letters off both ends, keeping interior apostrophes so
    // contractions survive ("don't" checks as one word).
    while (start < end && !isLetter(text.charAt(start))) start++;
    while (end > start && !isLetter(text.charAt(end - 1))) end--;

    const word = text.slice(start, end);
    if (word && isCheckable(raw, word)) {
      tokens.push({ word, start, end });
    }
  }

  return tokens;
}

/**
 * The word the user right-clicked, or null when that spot isn't checkable.
 *
 * A single-word selection wins (right-clicking inside a selection leaves it
 * intact); otherwise the caret decides. Deliberately does NOT reuse
 * `getWordRange`: that one breaks on spaces only, so a word after a Shift+Enter
 * newline would come back with a range spanning the newline.
 */
export function getSpellcheckTarget(
  text: string,
  selectionStart: number,
  selectionEnd: number,
): SpellToken | null {
  const tokens = tokenizeForSpellcheck(text);

  // Selection first: find the token that covers it exactly enough to be "the
  // word the user highlighted".
  if (selectionEnd > selectionStart) {
    const selected = tokens.find(
      (t) => selectionStart >= t.start && selectionEnd <= t.end,
    );
    return selected ?? null;
  }

  // Caret: `<= end` so a caret parked right after the last letter still counts.
  return tokens.find((t) => selectionStart >= t.start && selectionStart <= t.end) ?? null;
}

/**
 * Replace a range inside a controlled input/textarea so React sees the change.
 *
 * Assigning `.value` directly is swallowed by React's synthetic event system —
 * React caches the last value it set on the node, sees no difference, and never
 * fires onChange, so the component's state silently diverges from the DOM. Going
 * through the prototype's value setter and dispatching a bubbling `input` event
 * is the supported way to write into a controlled field from outside React.
 */
export function replaceInputRange(
  el: HTMLElement,
  start: number,
  end: number,
  text: string,
): void {
  el.focus();

  if (!(el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement)) {
    // contenteditable and friends: no value property to drive, so fall back to
    // the editing command.
    document.execCommand('insertText', false, text);
    return;
  }

  const setter = Object.getOwnPropertyDescriptor(
    Object.getPrototypeOf(el),
    'value',
  )?.set;

  if (!setter) {
    el.setSelectionRange(start, end);
    document.execCommand('insertText', false, text);
    return;
  }

  const current = el.value;
  setter.call(el, current.slice(0, start) + text + current.slice(end));
  el.dispatchEvent(new Event('input', { bubbles: true }));

  const caret = start + text.length;
  el.setSelectionRange(caret, caret);
}

/**
 * Every version of `word` with one adjacent pair of letters swapped.
 *
 *   "teh" -> ["eth", "the"]
 *
 * Hunspell-style suggestion (and nspell's implementation of it) builds its
 * candidates from the replacement table, keyboard-adjacent substitutions and
 * doubled letters. None of those produce a transposition, so the single most
 * common English typo class — "teh", "adn", "taht", "waht", "liek" — comes back
 * with no useful suggestion at all. Measured over 22 common typos, folding these
 * in took the top-5 hit rate from 15 to 21; generating the rest of the
 * edit-distance-1 neighbourhood on top added nothing.
 *
 * Case is carried along by the slicing, so "Teh" yields "The".
 */
export function adjacentTranspositions(word: string): string[] {
  const out: string[] = [];
  for (let i = 0; i < word.length - 1; i++) {
    out.push(word.slice(0, i) + word[i + 1] + word[i] + word.slice(i + 2));
  }
  return out;
}
