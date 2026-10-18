/** Does an optimistic chat line's payload match the server's echo of it?
 *  Trailing whitespace is ignored on both sides: Twitch never echoes it (and
 *  the Rust parser trim_end()s the payload) while the compose box routinely
 *  carries it ("emote " from the picker). The duplicate-bypass suffix ends in
 *  U+E0000, which is not whitespace, so a bypassed repeat still has to match
 *  exactly. Pure so it can be unit-tested without the store's window access. */
export function sameSentContent(local: string, server: unknown): boolean {
  if (typeof server !== 'string') return false;
  return local.trimEnd() === server.trimEnd();
}

/** What "Bypass duplicate" adds to a repeated line. Twitch keeps U+E0000 in
 *  chat text (it strips the other tag characters), so the copy arrives
 *  different from the line before it. Same suffix as `/spam`'s VARY_SUFFIX. */
export const DUPLICATE_BYPASS_SUFFIX = ' \u{E0000}';

/** The text to send for `text` when the line sent just before it in the same
 *  chat was `lastSent`. Twitch compares a message only with the previous one,
 *  so repeats alternate plain, varied, plain. Varying every repeat made the
 *  third copy identical to the second, and Twitch dropped it. Kept in the page
 *  because the optimistic row is built from this exact text before the send
 *  leaves. */
export function varyRepeat(text: string, lastSent: string | undefined): string {
  if (lastSent === undefined) return text;
  const base = text.trimEnd();
  const lastVaried = lastSent.endsWith(DUPLICATE_BYPASS_SUFFIX);
  const lastBase = lastVaried ? lastSent.slice(0, -DUPLICATE_BYPASS_SUFFIX.length) : lastSent;
  if (lastBase.trimEnd() !== base) return text;
  return lastVaried ? base : base + DUPLICATE_BYPASS_SUFFIX;
}
