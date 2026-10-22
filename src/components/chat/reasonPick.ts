// What the reason picker does with a key and which saved reasons it numbers,
// kept apart from the component so both are tested without a DOM.

/** Longest reason Twitch takes on a ban or timeout. */
export const REASON_MAX = 500;

/** The saved reasons the picker numbers 1-9: trimmed, blanks and repeats
 *  dropped, the first nine in their saved order. */
export function pickerReasons(saved: readonly string[] | null | undefined): string[] {
  const out: string[] = [];
  for (const raw of saved ?? []) {
    const r = raw.trim().slice(0, REASON_MAX);
    if (r && !out.includes(r)) out.push(r);
    if (out.length === 9) break;
  }
  return out;
}

export type ReasonKeyResult =
  | { kind: 'pick'; reason: string | null }
  | { kind: 'close' }
  /** A letter typed outside the field: it starts a written reason. */
  | { kind: 'type' }
  | null;

/**
 * What a key does in the picker. Outside the text field a digit picks the
 * numbered reason (0 is "No reason") and any other character starts a
 * written one; in the field, Enter sends what is written. Esc always closes.
 */
export function reasonKey(
  key: string,
  ctx: { inField: boolean; text: string; reasons: readonly string[]; modified: boolean },
): ReasonKeyResult {
  if (key === 'Escape') return { kind: 'close' };
  if (ctx.inField) {
    if (key !== 'Enter') return null;
    const text = ctx.text.trim().slice(0, REASON_MAX);
    return text ? { kind: 'pick', reason: text } : null;
  }
  if (ctx.modified || key.length !== 1) return null;
  if (key === '0') return { kind: 'pick', reason: null };
  if (key >= '1' && key <= '9') {
    const reason = ctx.reasons[Number(key) - 1];
    return reason ? { kind: 'pick', reason } : null;
  }
  return key === ' ' ? null : { kind: 'type' };
}
