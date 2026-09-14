// Who's-live for FAVORITED channels, across every platform.
//
// Deliberately separate from `followsStore.liveByKey`. That store answers "who
// that I follow is live", and Home's Following tab and the sidebar's Followed
// section read it directly; folding favorites in would relabel a channel you
// merely favorited as one you follow.
//
// The backend `favorite_live_service` owns the liveness itself. It sweeps
// Twitch by user id, Kick and YouTube through their `live_check` adapters, and
// skips anything already covered by the follow poller so no channel is polled
// twice. TikTok registers no live check at all, so TikTok favorites never
// appear here; they live in the offline roster instead.
//
// Two ways in, exactly like the follows store: the `favorites-live-update`
// event, and a `get_favorite_live` pull at startup so the first sweep interval
// isn't spent looking empty.

import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import type { TwitchStream } from '../types';
import type { ProviderId } from '../types/providers';
import { makeKey } from '../utils/providerKey';
import { Logger } from '../utils/logger';

/** The row shape the backend emits. Twitch rows carry a BARE login as their key
 *  (matching `streamKey`), so a favorite row dedupes against the follow row for
 *  the same channel instead of rendering twice. */
export interface FavoriteStreamRow extends TwitchStream {
  provider: ProviderId;
  key: string;
  watch_url: string;
}

interface FavoritesLiveState {
  /** Live favorites keyed by the row's platform key. */
  liveByKey: Record<string, FavoriteStreamRow>;
  hydrated: boolean;
  hydrate: () => Promise<void>;
  /** Replace the whole snapshot (the event carries every source at once). */
  setLive: (streams: FavoriteStreamRow[]) => void;
}

/** True once an event has landed.
 *
 *  `hydrate` is only the INITIAL paint. It replaces the map wholesale, so if its
 *  request resolves after an event has already arrived it would overwrite fresh
 *  rows with the older snapshot it captured. One flag is enough here, unlike the
 *  follows store's per-provider set, because the backend emits the complete
 *  snapshot on every sweep rather than one source's slice.
 */
let liveFromEvent = false;

export const useFavoritesStore = create<FavoritesLiveState>((set) => ({
  liveByKey: {},
  hydrated: false,

  hydrate: async () => {
    try {
      const rows = await invoke<FavoriteStreamRow[]>('get_favorite_live');
      if (liveFromEvent) {
        set({ hydrated: true });
        return;
      }
      const next: Record<string, FavoriteStreamRow> = {};
      for (const row of rows ?? []) {
        next[row.key || makeKey(row.provider, row.user_login)] = row;
      }
      set({ liveByKey: next, hydrated: true });
    } catch (e) {
      Logger.debug('[favorites] live hydrate skipped:', e);
      set({ hydrated: true });
    }
  },

  setLive: (streams) => {
    liveFromEvent = true;
    const next: Record<string, FavoriteStreamRow> = {};
    for (const row of streams) {
      next[row.key || makeKey(row.provider, row.user_login)] = row;
    }
    set({ liveByKey: next, hydrated: true });
  },
}));
