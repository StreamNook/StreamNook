// Per-source live state for a MultiChat window, derived from what Rust already
// holds. Twitch rows read the channel-state store (one batched viewer poll in
// Rust for every watched channel); every other platform reads the channel meta
// its chat adapter captured. Nothing here fetches.
//
// Key spaces: rows are keyed by the composite `makeKey(provider, channel)`,
// the same key a MultiChat tab carries. The channel-state store is keyed by
// the bare, lowercased Twitch login and is only ever read with that login; the
// two never meet in one lookup.
import type { ProviderId } from '../../types/providers';
import { makeKey } from '../../utils/providerKey';

export interface LiveSource {
  channel: string;
  channelName?: string;
  provider?: ProviderId;
  /** Twitch user id, which a channel-state watch is keyed by. */
  channelId?: string | null;
}

/** The slice of a Twitch channel state this module reads. */
export interface TwitchLiveState {
  viewer_count: number | null;
  /** When Rust's viewer poll last answered; null until it has. */
  viewers_at: number | null;
}

/** What a non-Twitch chat adapter reports for its channel. */
export interface ProviderLiveMeta {
  viewer_count?: number | null;
  is_live?: boolean;
}

export interface SourceLive {
  /** `makeKey(provider, channel)`: the tab's own key. */
  key: string;
  name: string;
  provider: ProviderId;
  count: number | null;
  isLive: boolean;
  /** False until Rust has answered for this source at all. An unknown source
   *  shows neither live nor offline. */
  known: boolean;
}

export function sourceKey(src: LiveSource): string {
  return makeKey(src.provider ?? 'twitch', src.channel);
}

/** The bare login the channel-state store keys a Twitch source by. */
export function twitchStateLogin(src: LiveSource): string {
  return src.channel.trim().toLowerCase();
}

/**
 * One row per source, in source order.
 *
 * - `twitchStates`: the channel-state store's map, keyed by bare login.
 *   Rust reports a viewer count only while the channel is live.
 * - `providerMeta`: the last adapter answer per source, keyed by `sourceKey`;
 *   a key holding `null` means the adapter has not resolved the channel yet.
 */
export function liveStats(
  sources: LiveSource[],
  twitchStates: ReadonlyMap<string, TwitchLiveState>,
  providerMeta: Readonly<Record<string, ProviderLiveMeta | null>>,
): SourceLive[] {
  return sources.map((src) => {
    const provider = src.provider ?? 'twitch';
    const key = sourceKey(src);
    const name = src.channelName || src.channel;
    if (provider === 'twitch') {
      const state = twitchStates.get(twitchStateLogin(src));
      const count = state?.viewer_count ?? null;
      const known = state?.viewers_at != null;
      return { key, name, provider, count, isLive: known && count !== null, known };
    }
    const meta = providerMeta[key];
    if (!meta) return { key, name, provider, count: null, isLive: false, known: false };
    const isLive = meta.is_live === true;
    return { key, name, provider, count: isLive ? (meta.viewer_count ?? null) : null, isLive, known: true };
  });
}

/** Total viewers across the live sources. */
export function totalViewers(stats: SourceLive[]): number {
  return stats.reduce((sum, s) => sum + (s.isLive && s.count ? s.count : 0), 0);
}

/** The words a tab's accessible label carries for its live state. */
export function liveWord(live: SourceLive | undefined): 'Live' | 'Offline' | null {
  if (!live || !live.known) return null;
  return live.isLive ? 'Live' : 'Offline';
}
