// In-app follows for platforms whose own follow list we can't read.
//
// Twitch follows come from Helix and live in AppStore.followedStreams. Kick and
// TikTok expose no followed-channels API to us, and YouTube's subscriptions are
// only readable while a cookie session exists — so StreamNook keeps its own
// per-platform follow list, persisted by the backend in
// `Settings.provider_follows` (a TYPED Rust field, because the who's-live
// poller reads it too).
//
// Liveness arrives two ways, both feeding `liveByKey`: the backend poller's
// `provider-live-update` event, and an initial `get_provider_followed_live`
// pull at startup. Consumers read the merged view through
// `useUnifiedFollowedLive`.

import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import type { ProviderFollow, TwitchStream } from '../types';
import type { ProviderId } from '../types/providers';
import { makeKey, normalizeChannel } from '../utils/providerKey';
import { Logger } from '../utils/logger';

/** The row shape the backend emits for provider streams. */
export interface ProviderStreamRow extends TwitchStream {
  provider: ProviderId;
  key: string;
  watch_url: string;
}

interface FollowsState {
  follows: ProviderFollow[];
  /** Live rows keyed by composite `provider:channel`. */
  liveByKey: Record<string, ProviderStreamRow>;
  /** True once the follow list has been read from the backend. */
  hydrated: boolean;

  hydrate: () => Promise<void>;
  follow: (provider: ProviderId, channel: string, displayName?: string) => Promise<void>;
  unfollow: (provider: ProviderId, channel: string) => Promise<void>;
  isFollowed: (provider: ProviderId, channel: string) => boolean;
  /** True when the user subscribes to this channel on the platform. */
  isSubscribed: (provider: ProviderId, channel: string) => boolean;
  /** Import the real follow + subscription list from a Kick website session.
   *  `interactive` opens the sign-in window; otherwise it silently re-syncs an
   *  existing session. Returns how many channels were imported. */
  syncKick: (interactive: boolean) => Promise<{ imported: number; subscribed: number }>;
  /** Import the signed-in YouTube account's subscriptions as follows. */
  syncYouTube: () => Promise<{ imported: number }>;
  /** Back-fill follows for a connected platform that has none imported yet. */
  importMissingFollows: () => Promise<void>;
  /** Replace the live snapshot for one provider (from a poller event). */
  setProviderLive: (provider: ProviderId, streams: ProviderStreamRow[]) => void;
  /** Forget everything about a platform whose account just went away. */
  clearProvider: (provider: ProviderId) => void;
  /** Drop only a platform's LIVE rows, keeping its follow list. For sessions
   *  that died on their own where liveness can no longer be swept (YouTube):
   *  stale "live" rows would be a false reading, but the follows are still real. */
  clearProviderLive: (provider: ProviderId) => void;
  /** Pull the current who's-live snapshot for every provider. */
  refreshLive: () => Promise<void>;
}

/** Canonical form to STORE and send. Case policy is per-platform and lives in
 *  `providerKey`; this used to be a blanket `.toLowerCase()`, which meant that
 *  however carefully the backend preserved a YouTube `UC` id, the frontend had
 *  already flattened it before the call. */
const norm = (provider: ProviderId, channel: string) =>
  normalizeChannel(provider, channel.trim());

/** Loose comparison for READS. Rows written before the casing fix are lowercased
 *  on disk, so an exact match would report a followed channel as unfollowed.
 *  Writes stay canonical; only the comparison is forgiving. */
const sameChannel = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/** Providers whose live rows have arrived from the backend poller.
 *
 *  `refreshLive` is only the INITIAL paint — it exists so the sidebar isn't empty
 *  for the first poll interval. It replaces `liveByKey` wholesale, so if its
 *  request resolves after an event has landed, it would overwrite fresh rows with
 *  the older snapshot it captured. YouTube shows this up: its sweep is a slow
 *  authenticated browse, so on a cold start the snapshot is reliably YouTube-less
 *  at the moment the initial pull reads it.
 *
 *  Tracked PER PROVIDER, not as one global counter. Each provider is swept and
 *  emitted independently, so a fast Kick event must not suppress YouTube's
 *  initial paint — which is exactly what a global sequence would do. */
const liveFromEvent = new Set<ProviderId>();

export const useFollowsStore = create<FollowsState>((set, get) => ({
  follows: [],
  liveByKey: {},
  hydrated: false,

  hydrate: async () => {
    try {
      const follows = await invoke<ProviderFollow[]>('get_provider_follows');
      set({ follows: follows ?? [], hydrated: true });
    } catch (e) {
      // The commands land with the Kick phase; until then this is a no-op.
      Logger.debug('[follows] hydrate skipped:', e);
      set({ hydrated: true });
    }
    void get().importMissingFollows();
  },

  // Import a connected platform's follows when we hold a session but have NOTHING
  // imported from it.
  //
  // The import otherwise only runs at CONNECT time, so an account connected before
  // that import existed (or one whose rows were cleared) stays permanently empty:
  // the connection looks fine, the Following tab is bare, and nothing ever
  // reconciles the two. Gated on "connected AND zero imported rows", so it costs
  // one check at startup and stops firing as soon as it succeeds.
  importMissingFollows: async () => {
    try {
      const connected = await invoke<boolean>('youtube_is_connected');
      if (!connected) return;
      const hasImported = get().follows.some((f) => f.provider === 'youtube' && f.imported);
      if (hasImported) return;
      const { imported } = await get().syncYouTube();
      Logger.info(`[follows] back-filled ${imported} YouTube subscription(s)`);
    } catch (e) {
      // Never fatal: the app works fine with an un-imported connection.
      Logger.debug('[follows] back-fill skipped:', e);
    }
  },

  follow: async (provider, channel, displayName) => {
    const ch = norm(provider, channel);
    if (!ch || get().isFollowed(provider, ch)) return;
    // Optimistic: the row appears immediately, and a failed write rolls back.
    const optimistic: ProviderFollow = {
      provider,
      channel: ch,
      display_name: displayName,
      added_at: new Date().toISOString(),
    };
    set((s) => ({ follows: [...s.follows, optimistic] }));
    try {
      await invoke('provider_follow', { provider, channel: ch, displayName: displayName ?? null });
    } catch (e) {
      Logger.warn('[follows] follow failed:', e);
      set((s) => ({
        follows: s.follows.filter(
          (f) => !(f.provider === provider && sameChannel(f.channel, ch)),
        ),
      }));
    }
  },

  unfollow: async (provider, channel) => {
    const ch = norm(provider, channel);
    const previous = get().follows;
    // Loose match so a legacy lowercased row is actually removed. Exact matching
    // here is what made unfollowing an imported YouTube channel a silent no-op.
    set((s) => ({
      follows: s.follows.filter((f) => !(f.provider === provider && sameChannel(f.channel, ch))),
    }));
    try {
      await invoke('provider_unfollow', { provider, channel: ch });
    } catch (e) {
      Logger.warn('[follows] unfollow failed:', e);
      set({ follows: previous });
    }
  },

  isFollowed: (provider, channel) => {
    const ch = norm(provider, channel);
    return get().follows.some((f) => f.provider === provider && sameChannel(f.channel, ch));
  },

  isSubscribed: (provider, channel) => {
    const ch = norm(provider, channel);
    return get().follows.some(
      (f) => f.provider === provider && sameChannel(f.channel, ch) && f.subscribed,
    );
  },

  syncKick: async (interactive) => {
    const result = await invoke<{ imported: number; subscribed: number; follows: ProviderFollow[] }>(
      'kick_account_sync',
      { interactive },
    );
    set({ follows: result.follows ?? [] });
    // The poller re-reads the follow list every sweep, but pull the current
    // snapshot now so freshly imported channels appear without waiting a minute.
    void get().refreshLive();
    return { imported: result.imported, subscribed: result.subscribed };
  },

  syncYouTube: async () => {
    const result = await invoke<{ imported: number; follows: ProviderFollow[] }>(
      'youtube_account_sync',
    );
    set({ follows: result.follows ?? [] });
    // Same reason as syncKick: the poller re-reads the list every sweep, but pull
    // the snapshot now so imported channels appear without waiting for one.
    void get().refreshLive();
    return { imported: result.imported };
  },

  setProviderLive: (provider, streams) => {
    liveFromEvent.add(provider);
    set((s) => {
      // Replace this provider's slice wholesale so channels that went offline
      // drop out, and leave every other provider's rows untouched.
      const next: Record<string, ProviderStreamRow> = {};
      for (const [key, row] of Object.entries(s.liveByKey)) {
        if (row.provider !== provider) next[key] = row;
      }
      for (const row of streams) {
        next[row.key || makeKey(provider, row.user_login)] = row;
      }
      return { liveByKey: next };
    });
  },

  /**
   * Drop a platform's follows AND its live rows.
   *
   * Disconnecting used to leave the live snapshot untouched, so signing out of
   * Kick and switching to All platforms still showed every Kick channel that
   * had been live at the moment you signed out. `hydrate` did not help: it
   * replaces `follows`, and the sidebar and Home read `liveByKey`. Nor did the
   * poller, because with no follows left to sweep it has no reason to emit an
   * update for that platform, so the stale rows had nothing to clear them until
   * the next app start.
   *
   * `liveFromEvent` is cleared too, otherwise a later reconnect would find the
   * platform still marked as "already delivered by an event" and `refreshLive`
   * would skip repainting it.
   */
  clearProvider: (provider) => {
    liveFromEvent.delete(provider);
    set((s) => {
      const liveByKey: Record<string, ProviderStreamRow> = {};
      for (const [key, row] of Object.entries(s.liveByKey)) {
        if (row.provider !== provider) liveByKey[key] = row;
      }
      return { liveByKey, follows: s.follows.filter((f) => f.provider !== provider) };
    });
  },

  clearProviderLive: (provider) => {
    // `liveFromEvent` is cleared so a reconnect's initial `refreshLive` repaints
    // this provider instead of trusting a pre-death event delivery.
    liveFromEvent.delete(provider);
    set((s) => {
      const liveByKey: Record<string, ProviderStreamRow> = {};
      for (const [key, row] of Object.entries(s.liveByKey)) {
        if (row.provider !== provider) liveByKey[key] = row;
      }
      return { liveByKey };
    });
  },

  refreshLive: async () => {
    try {
      const rows = await invoke<ProviderStreamRow[]>('get_provider_followed_live');
      set((s) => {
        const next: Record<string, ProviderStreamRow> = {};
        // Keep everything an event already delivered: those rows are authoritative
        // and fresher than this snapshot, whichever finished first.
        for (const [key, row] of Object.entries(s.liveByKey)) {
          if (liveFromEvent.has(row.provider)) next[key] = row;
        }
        // Fill in only the providers still waiting on their first event.
        for (const row of rows ?? []) {
          if (liveFromEvent.has(row.provider)) continue;
          next[row.key || makeKey(row.provider, row.user_login)] = row;
        }
        return { liveByKey: next };
      });
    } catch (e) {
      Logger.debug('[follows] live refresh skipped:', e);
    }
  },
}));
