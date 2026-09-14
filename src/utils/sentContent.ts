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
