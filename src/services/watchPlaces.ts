// The categories and channels a badge names, resolved by Rust
// (services/watch_places.rs) into what their cards draw. Desktop's badge page
// and the phones' badge sheet both present this as it comes.

import { invoke } from '@tauri-apps/api/core';
import type { TwitchStream } from '../types';

export interface PlaceCategory {
  /** The name it was asked for. */
  query: string;
  /** Absent when the lookup failed and the name is kept as asked. */
  id: string | null;
  name: string;
  box_art_url: string | null;
  viewers_line: string | null;
}

export interface PlaceChannel {
  login: string;
  user_id: string | null;
  display_name: string;
  avatar_url: string | null;
  /** The live stream, for `startStream`. */
  live: TwitchStream | null;
  /** "Live · Just Chatting · 1.2K viewers", or "Offline". */
  status_line: string;
  /** The 7TV picture at the cards' size; the iPhone draws it, the web shells
   *  use their own 7TV avatar lookup. */
  picture?: { still: string; animated?: string };
}

export interface WatchPlaces {
  categories: PlaceCategory[];
  /** Live channels first. */
  channels: PlaceChannel[];
}

const EMPTY: WatchPlaces = { categories: [], channels: [] };

/** `limit` caps the channels Rust resolves (16 by default, at most 100). */
export function resolveWatchPlaces(categories: string[], channels: string[], limit?: number): Promise<WatchPlaces> {
  if (categories.length === 0 && channels.length === 0) return Promise.resolve(EMPTY);
  return invoke<WatchPlaces>('resolve_watch_places', { categories, channels, limit });
}
