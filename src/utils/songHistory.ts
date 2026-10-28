// Your songs: every song the user identified, kept by Rust
// (src-tauri/src/services/song_id/history.rs). This is the bridge: the shapes
// Rust sends, its calls and change event, and the one way any surface opens
// the list.
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useAppStore } from '../stores/AppStore';
import type { PlayerSource } from './activeVideo';
import type { SongProvider } from './songId';

export type SongSource = PlayerSource;

/** One row of the list. */
export interface SongRow {
  key: string;
  title: string;
  artist: string;
  /** The cover, a small copy when its host can draw one. */
  art: string | null;
  /** The full cover when `art` is a small copy, for a host that refused the size. */
  art_full?: string;
  last_ms: number;
  count: number;
  /** The latest place it was heard that named a channel. */
  heard: SongSource | null;
}

export interface SongSighting {
  at_ms: number;
  source: SongSource | null;
}

/** One song with everything its detail shows. */
export interface SongEntry {
  key: string;
  title: string;
  artist: string;
  album: string | null;
  album_art: string | null;
  shazam_url: string | null;
  song_link: string | null;
  providers: SongProvider[];
  /** `providers` are search pages, not the track's own links. */
  links_are_searches?: boolean;
  first_ms: number;
  last_ms: number;
  count: number;
  sightings: SongSighting[];
}

export interface SongPage {
  rows: SongRow[];
  matched: number;
  total: number;
  /** Places heard name more than one platform: rows then mark theirs. */
  mixed_platforms: boolean;
  /** Channels the songs held were heard on. */
  channels: number;
}

export type SongHistoryChange = { kind: 'filed'; key: string } | { kind: 'removed'; key: string } | { kind: 'cleared' };

export const fetchSongPage = (query: string, offset: number, limit: number): Promise<SongPage> =>
  invoke<SongPage>('get_song_history', { query: query || null, offset, limit });

export const fetchSong = (key: string): Promise<SongEntry | null> => invoke<SongEntry | null>('get_song', { key });

export const removeSong = (key: string): Promise<boolean> => invoke<boolean>('remove_song', { key });

/** Undo the last Remove, when `key` names its song. */
export const restoreSong = (key: string): Promise<boolean> => invoke<boolean>('restore_song', { key });

export const clearSongHistory = (): Promise<void> => invoke<void>('clear_song_history');

export const onSongHistoryChanged = (cb: (change: SongHistoryChange) => void): Promise<UnlistenFn> =>
  listen<SongHistoryChange>('song-history-changed', (event) => cb(event.payload));

// The surfaces that draw the list in this window: the desktop's panel host
// (main and MultiChat windows, and the legacy phone shell, which mounts the
// desktop app) and the phone's Songs screen. A window with none, like the
// stream overlay, leaves the chat card's link out rather than offering one
// that opens nothing.
const surfaces = { panel: 0, screen: 0 };
// Chat cards that show the Your songs control, told when a surface comes or
// goes (a card can draw in the same pass that mounts the surface).
const surfaceWatchers = new Set<() => void>();

function surfacesChanged(): void {
  for (const watcher of surfaceWatchers) watcher();
}

export function registerSongsSurface(kind: 'panel' | 'screen'): () => void {
  surfaces[kind] += 1;
  surfacesChanged();
  return () => {
    surfaces[kind] -= 1;
    surfacesChanged();
  };
}

/** For useSyncExternalStore: hear when this window gains or loses a surface
 *  that draws Your songs. */
export function subscribeSongsSurface(onChange: () => void): () => void {
  surfaceWatchers.add(onChange);
  return () => {
    surfaceWatchers.delete(onChange);
  };
}

export const hasSongsSurface = (): boolean => surfaces.panel + surfaces.screen > 0;

/** Open Your songs, on one song's detail when `focus` (its key) is given,
 *  wherever this window draws it: the phone's screen, else the desktop's panel. */
export function openSongs(focus: string | null = null): void {
  if (surfaces.screen > 0) {
    void import('../mobile/navStore').then(({ useMobileNavStore }) => useMobileNavStore.getState().openSongs(focus));
  } else if (surfaces.panel > 0) {
    useAppStore.getState().openSongs(focus);
  }
}
