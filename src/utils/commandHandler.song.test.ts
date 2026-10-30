// Android says "Listening to identify the song..." in chat for /song (the
// desktop shows its strip instead). The check for a listening already under
// way reads before the page action runs, which starts this command's own: the
// first /song keeps its line, a second while it listens says nothing.
import { beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({
  listening: null as unknown,
  noticeIn: vi.fn(),
}));

vi.mock('./platform', () => ({ IS_ANDROID: true, IS_MOBILE: true, IS_MAC: false }));
vi.mock('./slashUi', () => ({
  // The page action starts the listening, as recognizeNowPlaying does.
  applySlashUi: vi.fn(async () => {
    h.listening = { id: 'l1', phase: 'listening' };
  }),
  noticeIn: (...a: unknown[]) => h.noticeIn(...a),
}));
vi.mock('./songListening', () => ({ getListening: () => h.listening }));
vi.mock('../stores/AppStore', () => ({ useAppStore: { getState: () => ({ addToast: vi.fn() }) } }));
vi.mock('../stores/pinStore', () => ({ usePinStore: { getState: () => ({ requestRefresh: vi.fn() }) } }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

import { applySlashOutcome, type SlashOutcome } from './commandHandler';

const song: SlashOutcome = {
  handled: true,
  send: null,
  clear_draft: true,
  toasts: [],
  notices: ['Listening to identify the song...'],
  ui: { kind: 'identify_song' },
};

describe('/song on Android', () => {
  beforeEach(() => {
    h.listening = null;
    h.noticeIn.mockClear();
  });

  it('says it is listening the first time, though its own listening starts first', async () => {
    await applySlashOutcome(song, 'xqc', '1', () => {});
    expect(h.noticeIn).toHaveBeenCalledWith('xqc', 'Listening to identify the song...');
  });

  it('says nothing for a /song while a listening runs', async () => {
    h.listening = { id: 'l0', phase: 'listening' };
    await applySlashOutcome(song, 'xqc', '1', () => {});
    expect(h.noticeIn).not.toHaveBeenCalled();
  });

  it("prints other commands' lines as ever", async () => {
    h.listening = { id: 'l0', phase: 'listening' };
    await applySlashOutcome({ ...song, notices: ['Opened'], ui: { kind: 'open_songs' } }, 'xqc', '1', () => {});
    expect(h.noticeIn).toHaveBeenCalledWith('xqc', 'Opened');
  });
});
