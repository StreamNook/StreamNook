import { beforeEach, describe, expect, it, vi } from 'vitest';

const announce = vi.fn();
const open = vi.fn();
const recognize = vi.fn((..._a: unknown[]) => Promise.resolve({ status: 'no-match' }));
const remote = { listens: false, request: vi.fn(async (..._a: unknown[]) => {}) };
vi.mock('../stores/AppStore', () => ({ useAppStore: { getState: () => ({ addToast: vi.fn() }) } }));
vi.mock('../stores/chatConnectionStore', () => ({ injectSystemMessage: vi.fn(), systemSourceFor: () => undefined }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('./songId', () => ({
  recognizeNowPlaying: (...args: unknown[]) => recognize(...args),
  announceSong: (...args: unknown[]) => announce(...args),
}));
vi.mock('./songHistory', () => ({ openSongs: (...args: unknown[]) => open(...args) }));
vi.mock('./songListening', () => ({
  listensRemotely: () => remote.listens,
  requestRemoteIdentify: (...args: unknown[]) => remote.request(...args),
}));

import { applySlashUi } from './slashUi';

describe('song page actions', () => {
  beforeEach(() => {
    announce.mockClear();
    recognize.mockClear();
    remote.request.mockClear();
    remote.listens = false;
  });

  it('answers /song in the room it was typed in, and listens for that room', async () => {
    await applySlashUi({ kind: 'identify_song' }, 'kick:somechan', '');
    await new Promise((r) => setTimeout(r, 0));
    expect(recognize).toHaveBeenCalledWith(null, { room: 'kick:somechan' });
    expect(announce).toHaveBeenCalledWith({ status: 'no-match' }, 'kick:somechan');
  });

  it("asks the main window's player where this window has none", async () => {
    remote.listens = true;
    await applySlashUi({ kind: 'identify_song' }, 'xqc', '1');
    expect(remote.request).toHaveBeenCalledWith('xqc');
    expect(recognize).not.toHaveBeenCalled();
  });

  it('opens Your songs for /songs', async () => {
    await applySlashUi({ kind: 'open_songs' }, 'xqc', '1');
    expect(open).toHaveBeenCalledTimes(1);
  });
});
