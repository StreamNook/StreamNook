import { describe, expect, it, vi } from 'vitest';

const announce = vi.fn();
const open = vi.fn();
vi.mock('../stores/AppStore', () => ({ useAppStore: { getState: () => ({ addToast: vi.fn() }) } }));
vi.mock('../stores/chatConnectionStore', () => ({ injectSystemMessage: vi.fn(), systemSourceFor: () => undefined }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('./songId', () => ({
  recognizeNowPlaying: () => Promise.resolve({ status: 'no-match' }),
  announceSong: (...args: unknown[]) => announce(...args),
}));
vi.mock('./songHistory', () => ({ openSongs: (...args: unknown[]) => open(...args) }));

import { applySlashUi } from './slashUi';

describe('song page actions', () => {
  it('answers /song in the room it was typed in', async () => {
    await applySlashUi({ kind: 'identify_song' }, 'kick:somechan', '');
    await new Promise((r) => setTimeout(r, 0));
    expect(announce).toHaveBeenCalledWith({ status: 'no-match' }, 'kick:somechan');
  });

  it('opens Your songs for /songs', async () => {
    await applySlashUi({ kind: 'open_songs' }, 'xqc', '1');
    expect(open).toHaveBeenCalledTimes(1);
  });
});
