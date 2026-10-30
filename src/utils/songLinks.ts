// Opening a song's link, the one way every surface does it (the song page's
// "Listen on" pills and "Search on" logos, the chat's song card, the answer
// over the composer, the player's toast). On the desktop Rust opens it in the
// music app on this computer when one is installed (Cider or Apple Music for
// an Apple Music link, Spotify for a Spotify one), building the app's link
// from the web link it is given, and in the browser otherwise
// (src-tauri/src/services/song_id/app_links.rs). Phones open the web link,
// which the system hands to the service's app itself. Rust's list of the
// installed apps names the song page's buttons ("Opens in Cider").
import { useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import { Logger } from './logger';
import { openExternal } from './openExternal';
import { IS_MOBILE } from './platform';

/** A music app on this computer that opens one service's links (Rust's SongApp). */
export interface SongApp {
  /** The service it plays, as Rust names links ("Apple Music"). */
  service: string;
  /** The app's own name ("Cider"). */
  app: string;
  /** What open_song_link takes to open a link in this app. Opaque. */
  target: string;
  /** The app a plain open of this service's links uses. */
  default: boolean;
}

export interface OpenSongLinkOptions {
  /** Open in this app (a `target` from the list) instead of the service's default. */
  app?: string | null;
  /** Open the web link in the browser, whatever is installed. */
  browser?: boolean;
}

/** Opens a song's link, and says so when nothing would open it. */
export async function openSongLink(url: string, { app = null, browser = false }: OpenSongLinkOptions = {}): Promise<void> {
  if (!IS_MOBILE && !browser) {
    try {
      // Rust falls back to the browser itself; an error means it refused the
      // link or could not open even that.
      await invoke<boolean>('open_song_link', { url, app });
      return;
    } catch (e) {
      Logger.warn('[Songs] open_song_link failed, opening in the browser:', e);
    }
  }
  if (!(await openExternal(url))) useAppStore.getState().addToast("Couldn't open that link", 'error');
}

// ---------------------------------------------------------------- the installed apps

const NONE: readonly SongApp[] = [];
let apps: readonly SongApp[] = NONE;
let asking = false;
const listeners = new Set<() => void>();

const sameApps = (a: readonly SongApp[], b: readonly SongApp[]) => JSON.stringify(a) === JSON.stringify(b);

/** Asks Rust which music apps are installed (Your songs asks each time it
 *  opens; Rust keeps its answer for a minute). Phones have none. */
export function refreshSongApps(): void {
  if (IS_MOBILE || asking) return;
  asking = true;
  invoke<SongApp[]>('song_app_links')
    .then((list) => {
      if (!Array.isArray(list) || sameApps(list, apps)) return;
      apps = list;
      for (const listener of listeners) listener();
    })
    .catch((e: unknown) => Logger.warn('[Songs] could not list the music apps:', e))
    .finally(() => {
      asking = false;
    });
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** The installed music apps, as last listed. */
export function useSongApps(): readonly SongApp[] {
  return useSyncExternalStore(
    subscribe,
    () => apps,
    () => NONE,
  );
}

/** One service's apps, its default first. */
export function songAppsFor(list: readonly SongApp[], service: string): SongApp[] {
  const own = list.filter((a) => a.service === service);
  return [...own.filter((a) => a.default), ...own.filter((a) => !a.default)];
}

/** What a link's button says it does: the app it opens in ("Opens in
 *  Cider"), or the link itself when it opens in the browser. */
export function songLinkTip(url: string, apps: readonly SongApp[]): string {
  return apps[0] ? `Opens in ${apps[0].app}` : url;
}

// ---------------------------------------------------------------- layers over Your songs

/** A song link's own layer over Your songs (the QR dialog, the app menu)
 *  carries this attribute, so the panel leaves Escape to it. */
export const SONG_LAYER_ATTR = 'data-song-layer';

/** Whether a song link's layer is open in this page. */
export function songLayerOpen(doc: Pick<ParentNode, 'querySelector'> | null | undefined): boolean {
  return !!doc?.querySelector(`[${SONG_LAYER_ATTR}]`);
}
