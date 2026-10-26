/**
 * The moderated-user marks after one more timeout or ban. A user's mark only
 * matters while one of the rows it strikes is still in the buffer, so marks
 * whose rows have all scrolled out are dropped here; a still-visible user
 * keeps theirs. Returns a new Map (the store's copy-on-write signal).
 */
export function withClearedEntry<E extends { affectedMessageIds: Set<string> }>(
  contexts: Map<string, E>,
  presentIds: Set<string>,
  userId: string,
  entry: E,
): Map<string, E> {
  const next = new Map<string, E>();
  for (const [uid, e] of contexts) {
    if (uid === userId) continue;
    for (const id of e.affectedMessageIds) {
      if (presentIds.has(id)) {
        next.set(uid, e);
        break;
      }
    }
  }
  next.set(userId, entry);
  return next;
}
