// Per-source live state for a chat tab strip (MultiChat's tabs and title-bar
// viewer total, the MultiNook chat switcher), mapped from the rows Rust sends
// (services/chat_dock_live.rs). Nothing here fetches or decides liveness.
//
// Key space: sources and rows are both keyed by the composite
// `makeKey(provider, channel)`, the key a tab carries.
import type { ProviderId } from '../types/providers';
import { makeKey } from './providerKey';

export interface LiveSource {
  channel: string;
  channelName?: string;
  provider?: ProviderId;
  /** Twitch user id; Rust looks it up when absent. */
  channelId?: string | null;
}

/** The part of Rust's row this module reads. */
export interface LiveRow {
  live: boolean;
  viewer_count: number | null;
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

/** One row per source, in source order. A source with no row is unknown. */
export function liveStats(sources: LiveSource[], rows: Readonly<Record<string, LiveRow>>): SourceLive[] {
  return sources.map((src) => {
    const provider = src.provider ?? 'twitch';
    const key = sourceKey(src);
    const name = src.channelName || src.channel;
    const row = rows[key];
    if (!row) return { key, name, provider, count: null, isLive: false, known: false };
    return { key, name, provider, count: row.live ? row.viewer_count : null, isLive: row.live, known: true };
  });
}

/** Total viewers across the live sources. */
export function totalViewers(stats: SourceLive[]): number {
  return stats.reduce((sum, s) => sum + (s.isLive && s.count ? s.count : 0), 0);
}

/** The word a tab's tooltip and accessible label carry for its live state. */
export function liveWord(live: SourceLive | undefined): 'Live' | 'Offline' | null {
  if (!live || !live.known) return null;
  return live.isLive ? 'Live' : 'Offline';
}
