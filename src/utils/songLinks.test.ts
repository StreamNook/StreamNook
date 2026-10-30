// The one way a song link opens: Rust's open_song_link on the desktop (the
// installed music app, else the browser), the system's opener on a phone or
// when Rust refuses, and a word when nothing would open it. The installed
// apps are asked for once per ask, each service's default first.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const platform = vi.hoisted(() => ({ mobile: false }));
const addToast = vi.hoisted(() => vi.fn());
vi.mock('./platform', async (importOriginal) => {
  const real = await importOriginal<typeof import('./platform')>();
  return {
    ...real,
    get IS_MOBILE() {
      return platform.mobile;
    },
  };
});
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('./openExternal', () => ({ openExternal: vi.fn(async () => true) }));
vi.mock('../stores/AppStore', () => ({ useAppStore: { getState: () => ({ addToast }) } }));

import { invoke } from '@tauri-apps/api/core';
import { openExternal } from './openExternal';
import { openSongLink, refreshSongApps, songAppsFor, songLayerOpen, songLinkTip, type SongApp } from './songLinks';

const SPOTIFY = 'https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT';

beforeEach(() => {
  platform.mobile = false;
  vi.mocked(invoke).mockReset();
  vi.mocked(openExternal).mockReset().mockResolvedValue(true);
  addToast.mockReset();
});
afterEach(() => {
  vi.restoreAllMocks();
});

describe('opening a song link', () => {
  it('asks Rust on the desktop, for the default app or a chosen one', async () => {
    vi.mocked(invoke).mockResolvedValue(true);
    await openSongLink(SPOTIFY);
    expect(invoke).toHaveBeenCalledWith('open_song_link', { url: SPOTIFY, app: null });
    await openSongLink(SPOTIFY, { app: 'CiderCollective.Cider_a6qxe093bx5xj!App' });
    expect(invoke).toHaveBeenLastCalledWith('open_song_link', {
      url: SPOTIFY,
      app: 'CiderCollective.Cider_a6qxe093bx5xj!App',
    });
    expect(openExternal).not.toHaveBeenCalled();
  });

  it('opens the web link itself when Rust refuses it', async () => {
    vi.mocked(invoke).mockRejectedValue('not a song link');
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    await openSongLink(SPOTIFY);
    expect(openExternal).toHaveBeenCalledWith(SPOTIFY);
  });

  it('goes straight to the browser when asked, and on a phone', async () => {
    await openSongLink(SPOTIFY, { browser: true });
    platform.mobile = true;
    await openSongLink(SPOTIFY);
    expect(invoke).not.toHaveBeenCalled();
    expect(openExternal).toHaveBeenCalledTimes(2);
  });

  it('says so when nothing would open it', async () => {
    platform.mobile = true;
    vi.mocked(openExternal).mockResolvedValue(false);
    await openSongLink(SPOTIFY);
    expect(addToast).toHaveBeenCalledWith("Couldn't open that link", 'error');
  });
});

describe('the installed apps', () => {
  const cider: SongApp = { service: 'Apple Music', app: 'Cider', target: 'c!App', default: true };
  const apple: SongApp = { service: 'Apple Music', app: 'Apple Music', target: 'a!App', default: false };
  const spotify: SongApp = { service: 'Spotify', app: 'Spotify', target: 'spotify', default: true };

  it('gives a service’s apps with its default first', () => {
    expect(songAppsFor([apple, spotify, cider], 'Apple Music')).toEqual([cider, apple]);
    expect(songAppsFor([apple, spotify, cider], 'Spotify')).toEqual([spotify]);
    expect(songAppsFor([apple, spotify, cider], 'Tidal')).toEqual([]);
  });

  it('name the app a pill opens in, else the pill shows its link', () => {
    expect(songLinkTip('https://music.apple.com/us/song/1', [cider, apple])).toBe('Opens in Cider');
    expect(songLinkTip(SPOTIFY, [])).toBe(SPOTIFY);
  });

  it('are asked of Rust on the desktop, never on a phone', () => {
    vi.mocked(invoke).mockResolvedValue([]);
    platform.mobile = true;
    refreshSongApps();
    expect(invoke).not.toHaveBeenCalled();
    platform.mobile = false;
    refreshSongApps();
    expect(invoke).toHaveBeenCalledWith('song_app_links');
  });
});

describe('a layer over Your songs', () => {
  it('is found by its attribute', () => {
    expect(songLayerOpen({ querySelector: (q: string) => (q === '[data-song-layer]' ? ({} as Element) : null) })).toBe(true);
    expect(songLayerOpen({ querySelector: () => null })).toBe(false);
    expect(songLayerOpen(null)).toBe(false);
  });
});
