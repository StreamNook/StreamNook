// Which label a favorite control shows, and the keys it is tracked by. Pure,
// so the store and the door (utils/favoriteDoor.ts) share it with no cycle.
//
// Rust owns the favorite lists. Every shell labels a row from what Rust last
// pushed (`following.favorite_ids` and `favorites_list`) and, while a press is
// on its way, from the press itself (`pending`).

import type { FavoriteListEntry } from '../types';
import type { ProviderId } from '../types/providers';
import { favoriteIdOf } from './favorites';

/** Settings keys only Rust writes. `updateSettings` never sends them. */
export const RUST_ONLY_KEYS = ['favorite_streamers', 'favorite_channels'] as const;

/** A channel as the favorite door takes it: the row's own fields. */
export interface FavoriteRef {
  provider: string;
  userId?: string | null;
  userLogin: string;
  userName?: string | null;
  avatar?: string | null;
}

/** Rust's answer to `set_favorite_channel`. */
export interface FavoriteAnswer {
  favorite: boolean;
  id: string;
}

/** A press on its way, per key. `answered` turns on once Rust answered the
 *  latest press and its answer matched; the entry then waits for the push. */
export interface FavoritePending {
  wanted: boolean;
  seq: number;
  answered: boolean;
}

export interface FavoriteLabelState {
  pending: Record<string, FavoritePending>;
  alias: Record<string, string>;
  ids: readonly string[];
  list: readonly FavoriteListEntry[];
}

/** The favorite id the row names, or null (a Twitch room before its id is
 *  known, a YouTube row known only by its video). */
export function favoriteIdOfRef(ref: FavoriteRef): string | null {
  return favoriteIdOf({
    provider: ref.provider as ProviderId,
    user_id: ref.userId ?? '',
    user_login: ref.userLogin,
  });
}

/** `id:<favorite id>` when the row names one, else
 *  `login:<provider>:<lowercased login>`. Two kinds, never mixed bare. */
export function favoriteKeyOf(ref: FavoriteRef): string {
  const id = favoriteIdOfRef(ref);
  if (id) return `id:${id}`;
  return `login:${ref.provider}:${ref.userLogin.trim().toLowerCase()}`;
}

/** What Rust last pushed says about this row. */
function storedFavorite(ref: FavoriteRef, key: string, state: FavoriteLabelState): boolean {
  const named = state.alias[key] ?? (key.startsWith('id:') ? key : null);
  if (named) return state.ids.includes(named.slice(3));
  // No id yet: the kept channel names it.
  const login = ref.userLogin.trim().toLowerCase();
  return state.list.some(
    (e) => e.provider === ref.provider && e.channel.toLowerCase() === login && state.ids.includes(e.id),
  );
}

/** The label for a row: the press on its way, else Rust's lists. */
export function labelFavorite(ref: FavoriteRef, state: FavoriteLabelState): boolean {
  const key = favoriteKeyOf(ref);
  const aliased = state.alias[key];
  const pending = state.pending[key] ?? (aliased ? state.pending[aliased] : undefined);
  if (pending) return pending.wanted;
  return storedFavorite(ref, key, state);
}

/** Drops answered presses that Rust's pushed ids now agree with. */
export function settlePending(
  pending: Record<string, FavoritePending>,
  ids: readonly string[],
): Record<string, FavoritePending> {
  let next: Record<string, FavoritePending> | null = null;
  for (const [key, p] of Object.entries(pending)) {
    if (!p.answered || !key.startsWith('id:')) continue;
    if (ids.includes(key.slice(3)) !== p.wanted) continue;
    next ??= { ...pending };
    delete next[key];
  }
  return next ?? pending;
}

/** Whether two pushed favorite views differ (both lists are short). */
export function favoritesDiffer(
  a: { ids: readonly string[]; list: readonly FavoriteListEntry[] },
  b: { ids: readonly string[]; list: readonly FavoriteListEntry[] },
): boolean {
  return JSON.stringify(a.ids) !== JSON.stringify(b.ids) || JSON.stringify(a.list) !== JSON.stringify(b.list);
}
