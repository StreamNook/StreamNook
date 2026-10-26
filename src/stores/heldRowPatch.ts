// Rows a chat window holds but has not drawn yet (a join's backlog wait, the
// next frame's queue) can be named by a re-stamp from Rust before they reach
// the slice. This swaps them where they sit.

/** Replace, in place, each row of `held` whose id `byId` names, and drop that
 *  id from `byId` so whatever is left is still unplaced. */
export function patchHeldRows<R>(
  held: unknown[],
  byId: Map<string, R>,
  patch: (row: { id: string }, update: R) => unknown,
): void {
  for (let i = 0; i < held.length && byId.size > 0; i++) {
    const m = held[i];
    if (typeof m !== 'object' || m === null) continue;
    const id = (m as { id?: unknown }).id;
    if (typeof id !== 'string') continue;
    const update = byId.get(id);
    if (update === undefined) continue;
    held[i] = patch(m as { id: string }, update);
    byId.delete(id);
  }
}
