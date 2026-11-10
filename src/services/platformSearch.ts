import { Channel, invoke } from '@tauri-apps/api/core';
import type { TwitchStream } from '../types';
import type { ProviderId } from '../types/providers';

/** One platform's search results. */
export interface SearchBatch {
  provider: ProviderId;
  /** Rows in the shape the page already renders for search results, only the
   *  ones that name the query, best match first. */
  streams: TwitchStream[];
  /** Each row's match strength, 0 best, on one scale for every platform. */
  ranks: number[];
  /** Set when the platform failed, as opposed to matching nothing. */
  error: string | null;
}

/**
 * Search `providers` at once, in Rust. `onBatch` gets each platform's rows the
 * moment that platform answers, so a slow or failing one never holds back the
 * rest. Resolves once every platform has answered.
 */
export function searchPlatforms(
  query: string,
  providers: ProviderId[],
  onBatch: (batch: SearchBatch) => void,
): Promise<void> {
  const channel = new Channel<SearchBatch>();
  channel.onmessage = onBatch;
  return invoke('search_platforms', { query, providers, onBatch: channel });
}

/**
 * Every platform's rows as one list: best match first whichever platform it is
 * on (so the channel itself leads), then live, then most watched, with
 * `providers` order breaking ties.
 */
export function mergeSearchBatches(
  providers: ProviderId[],
  found: ReadonlyMap<ProviderId, SearchBatch>,
): TwitchStream[] {
  const rows = providers.flatMap((pv, order) => {
    const batch = found.get(pv);
    if (!batch) return [];
    return batch.streams.map((stream, i) => ({ stream, order, rank: batch.ranks[i] ?? 99 }));
  });
  rows.sort(
    (a, b) =>
      a.rank - b.rank ||
      Number(!!b.stream.is_live) - Number(!!a.stream.is_live) ||
      (b.stream.viewer_count ?? 0) - (a.stream.viewer_count ?? 0) ||
      a.order - b.order,
  );
  return rows.map((r) => r.stream);
}
