// Desktop's and Android's door to a channel's favorite.
//
// Rust edits the lists in place (`set_favorite_channel`). Every heart, menu
// row and command sets the state it shows (`on`), never a blind toggle,
// through one chain, so presses reach Rust in the order they were made. The
// label comes from Rust's pushed lists and, while a press is on its way, from
// the press itself, so it flips at once and a double press toggles twice.

import { useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { TwitchStream } from '../types';
import { ensureHomeSnapshotSync, useAppStore, type AppState } from '../stores/AppStore';
import { Logger } from './logger';
import { streamProvider } from './streamProvider';
import {
  favoriteKeyOf,
  labelFavorite,
  settlePending,
  type FavoriteAnswer,
  type FavoriteLabelState,
  type FavoriteRef,
} from './favoriteLabel';

export { favoriteKeyOf, RUST_ONLY_KEYS } from './favoriteLabel';
export type { FavoriteAnswer, FavoriteRef } from './favoriteLabel';

const UNNAMED = "Couldn't work out which channel this is";

let chain: Promise<unknown> = Promise.resolve();
const seqs = new Map<string, number>();
let followingAsked = false;

/** The label rules need Rust's lists; a window that never mounted a Home
 *  still asks for them once. */
function ensureFollowingLoaded(): void {
  if (followingAsked) return;
  followingAsked = true;
  void ensureHomeSnapshotSync();
}

function labelStateOf(s: AppState): FavoriteLabelState {
  return {
    pending: s.favoritePending,
    alias: s.favoriteAlias,
    ids: s.following.favorite_ids ?? [],
    list: s.following.favorites_list ?? [],
  };
}

/** The row's fields, as the door takes them. */
export function favoriteRefOf(
  stream: Pick<TwitchStream, 'provider' | 'user_id' | 'user_login' | 'user_name' | 'profile_image_url'>,
): FavoriteRef {
  return {
    provider: streamProvider(stream),
    userId: stream.user_id || null,
    userLogin: stream.user_login,
    userName: stream.user_name || null,
    avatar: stream.profile_image_url || null,
  };
}

/** The label right now. For render paths that already re-render on
 *  `favoriteVersion` (Home, Sidebar) or on every store write. */
export function isFavoriteNow(ref: FavoriteRef): boolean {
  return labelFavorite(ref, labelStateOf(useAppStore.getState()));
}

/** The label, re-rendering when it changes. */
export function useIsFavorite(ref: FavoriteRef | null): boolean {
  useEffect(() => ensureFollowingLoaded(), []);
  return useAppStore((s) => (ref ? labelFavorite(ref, labelStateOf(s)) : false));
}

/** Whether Rust's lists have arrived in this window: before that a row keeps
 *  its place and waits. */
export function useFavoritesReady(): boolean {
  useEffect(() => ensureFollowingLoaded(), []);
  return useAppStore((s) => s.followingApplied);
}

/** Favorite (`on`) or unfavorite one channel. Answers the stored state, or
 *  null when Rust could not name the channel or the write failed. */
export function setFavoriteChannel(ref: FavoriteRef, on: boolean): Promise<boolean | null> {
  ensureFollowingLoaded();
  const key = favoriteKeyOf(ref);
  const seq = (seqs.get(key) ?? 0) + 1;
  seqs.set(key, seq);
  useAppStore.setState((s) => ({
    favoritePending: { ...s.favoritePending, [key]: { wanted: on, seq, answered: false } },
    favoriteVersion: s.favoriteVersion + 1,
  }));
  const run = chain.then(() =>
    invoke<FavoriteAnswer | null>('set_favorite_channel', {
      provider: ref.provider,
      userId: ref.userId ?? null,
      userLogin: ref.userLogin,
      userName: ref.userName ?? null,
      avatar: ref.avatar ?? null,
      favorite: on,
    }),
  );
  chain = run.catch(() => undefined);
  return run.then(
    (answer) => {
      settle(key, seq, on, answer ?? null, false);
      return answer ? answer.favorite : null;
    },
    (err: unknown) => {
      Logger.error('[favorites] could not save the favorite:', err);
      settle(key, seq, on, null, true);
      return null;
    },
  );
}

/** Applies one answer to the press it belongs to. An answer for an older
 *  press of the same key is dropped: a newer press owns the label. */
function settle(key: string, seq: number, wanted: boolean, answer: FavoriteAnswer | null, failed: boolean): void {
  const held = useAppStore.getState().favoritePending[key];
  if (!held || held.seq !== seq) return;
  if (!answer && !failed) useAppStore.getState().addToast(UNNAMED, 'error');
  useAppStore.setState((s) => {
    const pending = { ...s.favoritePending };
    const alias = { ...s.favoriteAlias };
    delete pending[key];
    if (answer && answer.favorite === wanted) {
      const stored = `id:${answer.id}`;
      // Rust named the channel by another id (a YouTube video row, a Twitch
      // room by login): the row reads that id from now on, and the press waits
      // there for the push.
      if (stored !== key) alias[key] = stored;
      if (!pending[stored]) pending[stored] = { wanted, seq: 0, answered: true };
    }
    return {
      favoritePending: settlePending(pending, s.following.favorite_ids ?? []),
      favoriteAlias: alias,
      favoriteVersion: s.favoriteVersion + 1,
    };
  });
}

/** Tests only: forget the chain and counters between cases. */
export function resetFavoriteDoorForTests(): void {
  chain = Promise.resolve();
  seqs.clear();
  followingAsked = true;
}
