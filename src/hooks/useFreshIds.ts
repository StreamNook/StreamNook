import { useState } from 'react';

const NONE: ReadonlySet<string> = new Set();

/** The ids in `ids` that `before` (a list's `\n`-joined signature) lacked. */
export function freshAgainst(before: string, ids: readonly string[]): ReadonlySet<string> {
  const had = new Set(before ? before.split('\n') : []);
  return new Set(ids.filter((id) => !had.has(id)));
}

/**
 * Which of `ids` just arrived: present now, absent the last time the list
 * changed. Nothing is fresh on the first render, so a preview that opens with
 * your loadout stays still and only the cosmetic you just picked plays its
 * swap. The set holds until the list changes again; an element keyed by its
 * id mounts once, so its entrance animation runs once.
 */
export function useFreshIds(ids: readonly string[]): ReadonlySet<string> {
  const sig = ids.join('\n');
  const [state, setState] = useState(() => ({ sig, fresh: NONE }));
  if (state.sig === sig) return state.fresh;
  const fresh = freshAgainst(state.sig, ids);
  setState({ sig, fresh });
  return fresh;
}
