import { Channel, invoke } from '@tauri-apps/api/core';
import type { TwitchStream } from '../types';
import type { ProviderId } from '../types/providers';

/** One platform's search results. */
export interface SearchBatch {
  provider: ProviderId;
  /** Rows in the shape the page already renders for search results. */
  streams: TwitchStream[];
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
