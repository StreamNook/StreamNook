import { beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({
  invoke: vi.fn(),
  app: {} as Record<string, unknown>,
  multi: { isMultiNookActive: false, toggleMultiNook: vi.fn() },
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => h.invoke(...a) }));
vi.mock('../stores/AppStore', () => ({ useAppStore: { getState: () => h.app } }));
vi.mock('../stores/multiNookStore', () => ({ usemultiNookStore: { getState: () => h.multi } }));
vi.mock('../utils/logger', () => ({ Logger: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() } }));

const { captureResumeSnapshot, resumePreviousSession } = await import('./sessionResume');
const { clearPauseIntent, heldPause, notePauseIntent } = await import('../utils/pausePark');

const liveApp = () => ({
  currentStream: { user_login: 'Streamer' },
  currentMediaType: 'live',
  originalMediaUrl: null,
  dropProgressActive: false,
  liveDropProgress: null,
});

beforeEach(() => {
  h.invoke.mockReset();
  h.multi.isMultiNookActive = false;
  h.multi.toggleMultiNook.mockReset();
  clearPauseIntent('main');
});

describe('resume snapshot', () => {
  it('records a paused live stream', async () => {
    h.app = liveApp();
    notePauseIntent('main', 'twitch:streamer');
    await captureResumeSnapshot();
    expect(h.invoke.mock.calls[0][0]).toBe('save_resume_snapshot');
    expect(h.invoke.mock.calls[0][1].snapshot).toMatchObject({
      stream_login: 'streamer',
      paused: true,
      multinook_active: false,
    });
  });

  it("a pause kept for another channel is not this stream's", async () => {
    h.app = liveApp();
    notePauseIntent('main', 'twitch:someone-else');
    await captureResumeSnapshot();
    expect(h.invoke.mock.calls[0][1].snapshot.paused).toBe(false);
  });

  it('records MultiNook with no main stream', async () => {
    h.app = { currentStream: null, currentMediaType: null };
    h.multi.isMultiNookActive = true;
    await captureResumeSnapshot();
    expect(h.invoke.mock.calls[0][1].snapshot).toMatchObject({ stream_login: null, paused: false, multinook_active: true });
  });

  it('nothing to resume writes nothing', async () => {
    h.app = { currentStream: null, currentMediaType: null };
    await captureResumeSnapshot();
    expect(h.invoke).not.toHaveBeenCalled();
  });

  it('a paused stream is held before it opens', async () => {
    let heldWhenOpened: unknown = 'not called';
    h.app = {
      startStream: vi.fn(async () => {
        heldWhenOpened = heldPause('main', 'twitch:streamer');
      }),
    };
    h.invoke.mockResolvedValueOnce({ stream_login: 'streamer', was_running: false, paused: true });
    await resumePreviousSession();
    expect(heldWhenOpened).toMatchObject({ key: 'twitch:streamer' });
  });

  it('MultiNook reopens', async () => {
    const startStream = vi.fn();
    h.app = { startStream };
    h.invoke.mockResolvedValueOnce({ stream_login: null, was_running: false, multinook_active: true });
    await resumePreviousSession();
    expect(h.multi.toggleMultiNook).toHaveBeenCalledTimes(1);
    expect(startStream).not.toHaveBeenCalled();
  });
});
