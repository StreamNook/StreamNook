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
/**
 * Put `insert` in place of the selection `start..end` (a bare caret when they
 * match), spaced from its neighbours: a space before it unless the text
 * before already ends in whitespace (or is empty), and one after unless the
 * text after already starts with a space. Returns the new text and the caret
 * just past the inserted word and its trailing space. Out-of-range positions
 * are clamped, so a stale selection can never cut the text.
 *
 *   ("hi|", "Kappa")         -> "hi Kappa |"
 *   ("|there", "Kappa")      -> "Kappa |there"
 *   ("a | b", "Kappa")       -> "a Kappa | b"
 */
export function insertAtCaret(
  value: string,
  start: number,
  end: number,
  insert: string,
): { text: string; caret: number } {
  const at = Math.max(0, Math.min(start, value.length));
  const to = Math.max(at, Math.min(end, value.length));
  const before = value.slice(0, at);
  const after = value.slice(to);
  const lead = before && !/\s$/.test(before) ? ' ' : '';
  const trail = after.startsWith(' ') ? '' : ' ';
  const head = before + lead + insert + trail;
  return { text: head + after, caret: head.length };
}

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
  /** What goes into the message when it differs from `name`. */
  insertText?: string;
  /** Where the emote comes from ("Twitch Global", "7TV", ...). */
  sourceLabel?: string;
  /** The same without the provider, shown beside its logo ("Global", "Sub"). */
  sourceDetail?: string;
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

/** The most candidates one Tab cycle walks through. */
export const TAB_CYCLE_LIMIT = 50;

/**
 * Chatter names after the emote matches, as the Tab cycle offers them. The
 * emotes arrive ranked from Rust; chatters always follow them. A `:` query is
 * emote-only, and an `@` query completes to `@name`.
 */
export function withChatterCandidates(
  emotes: EmoteTabCandidate[],
  word: string,
  users: ReadonlyArray<{ username: string; displayName?: string }>,
  mode: EmoteTabMatchMode,
): EmoteTabCandidate[] {
  const out = emotes.slice(0, TAB_CYCLE_LIMIT);
  if (word.startsWith(':') || out.length >= TAB_CYCLE_LIMIT) return out;
  const isAt = word.startsWith('@');
  const q = (isAt ? word.slice(1) : word).toLowerCase();
  if (!q) return out;
  const test = (s: string) => {
    const t = s.toLowerCase();
    return mode === 'starts_with' ? t.startsWith(q) : t.includes(q);
  };
  const seen = new Set(out.map((c) => c.name.toLowerCase()));
  for (const u of users) {
    const dn = u.displayName || u.username;
    if (seen.has(dn.toLowerCase())) continue;
    if (!test(dn) && !test(u.username)) continue;
    seen.add(dn.toLowerCase());
    out.push({
      name: (isAt ? '@' : '') + dn,
      priority: TAB_CYCLE_LIMIT,
      chatter: { username: u.username, displayName: dn },
    });
    if (out.length >= TAB_CYCLE_LIMIT) break;
  }
  return out;
}

/**
 * The part of the current word that sits before the caret. Any whitespace ends
 * a word here (unlike `getWordRange`), so a word after a Shift+Enter newline
 * starts at the newline.
 */
export function wordBeforeCaret(value: string, caret: number): { start: number; text: string } {
  let start = caret;
  while (start > 0 && !/\s/.test(value.charAt(start - 1))) start--;
  return { start, text: value.slice(start, caret) };
}

/** The DOM id of one emote-list option, for the textarea's aria-activedescendant. */
export const emoteOptionId = (listId: string, index: number) => `${listId}-opt-${index}`;

/** Move `index` by `delta` through a list of `length`, wrapping both ways. */
export function wrapIndex(index: number, delta: number, length: number): number {
  if (length <= 0) return 0;
  return (((index + delta) % length) + length) % length;
}

/**
 * The emote-list query a `:` opens, or null. The colon has to start a word and
 * be followed by at least two characters, the first a letter or digit, all
 * before the caret. That keeps times (`12:30`), emoticons (`:)`, `:D`) and
 * links (`https://`) from ever opening it.
 *
 *   "hi :lo|"  -> { anchor: 3, query: 'lo' }
 *   "12:30|"   -> null
 */
export function emoteSearchTrigger(
  value: string,
  caret: number,
): { anchor: number; query: string } | null {
  const { start, text: typed } = wordBeforeCaret(value, caret);
  if (!typed.startsWith(':')) return null;
  const query = typed.slice(1);
  if (query.length < 2 || !/^[\p{L}\p{N}_]/u.test(query)) return null;
  return { anchor: start, query };
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
