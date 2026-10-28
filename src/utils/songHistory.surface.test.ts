import { describe, expect, it, vi } from 'vitest';

const openPanel = vi.fn();
vi.mock('../stores/AppStore', () => ({ useAppStore: { getState: () => ({ openSongs: openPanel }) } }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

import { hasSongsSurface, openSongs, registerSongsSurface, subscribeSongsSurface } from './songHistory';

describe('where Your songs can open', () => {
  it('tells a watcher when a surface arrives and when it leaves', () => {
    const seen: boolean[] = [];
    const stop = subscribeSongsSurface(() => seen.push(hasSongsSurface()));
    expect(hasSongsSurface()).toBe(false);
    const leave = registerSongsSurface('panel');
    leave();
    stop();
    registerSongsSurface('panel')();
    expect(seen).toEqual([true, false]);
  });

  it('opens the panel where there is one, and nothing where there is none', () => {
    openSongs('k');
    expect(openPanel).not.toHaveBeenCalled();
    const leave = registerSongsSurface('panel');
    openSongs('k');
    expect(openPanel).toHaveBeenCalledWith('k');
    leave();
  });
});
