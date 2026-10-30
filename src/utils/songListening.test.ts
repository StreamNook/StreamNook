// One listening per window, shown to the strip and the note, with the level
// on its own feed; and the cross-window path for /song typed where no player
// is: the asking window shows the main window's listening and posts its
// answer, calls it off when nobody answers, and lets it go when the answer
// never comes. Every listener is the window's own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

type Handler = (e: { payload: unknown }) => void;
const h = vi.hoisted(() => ({
  label: 'main',
  handlers: new Map<string, Set<(e: { payload: unknown }) => void>>(),
  emitTo: vi.fn(async (_target: string, _event: string, _payload?: unknown) => {}),
  announce: vi.fn(),
  recognize: vi.fn(),
  video: null as unknown,
}));

vi.mock('@tauri-apps/api/event', () => ({
  emitTo: (target: string, event: string, payload?: unknown) => h.emitTo(target, event, payload),
}));
vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({
    label: h.label,
    listen: async (event: string, cb: Handler) => {
      const set = h.handlers.get(event) ?? new Set<Handler>();
      set.add(cb);
      h.handlers.set(event, set);
      return () => set.delete(cb);
    },
  }),
}));
vi.mock('./songId', () => ({
  announceSong: (...a: unknown[]) => h.announce(...a),
  recognizeNowPlaying: (...a: unknown[]) => h.recognize(...a),
}));
vi.mock('./activeVideo', () => ({ getActiveVideo: () => h.video }));
vi.mock('./platform', () => ({ IS_MOBILE: false }));
vi.mock('./logger', () => ({ Logger: { warn: vi.fn(), debug: vi.fn(), error: vi.fn(), info: vi.fn() } }));

type Mod = typeof import('./songListening');
let L: Mod;

const fire = (event: string, payload: unknown) => {
  for (const cb of [...(h.handlers.get(event) ?? [])]) cb({ payload });
};
const listeners = (event: string) => h.handlers.get(event)?.size ?? 0;
const sent = (event: string) => h.emitTo.mock.calls.filter((c) => c[1] === event);

beforeEach(async () => {
  vi.resetModules();
  vi.useFakeTimers();
  vi.setSystemTime(1_000_000);
  h.label = 'main';
  h.handlers.clear();
  h.emitTo.mockClear();
  h.announce.mockClear();
  h.recognize.mockReset();
  h.video = null;
  L = await import('./songListening');
});
afterEach(() => {
  vi.useRealTimers();
});

const begin = (remote: { window: string; id: string } | null = null) =>
  L.beginListening({ room: 'XQC', attempts: 2, captureMs: 5000, remote });

describe('a listening in this window', () => {
  it('shows one at a time: a second is busy', () => {
    const seen: (string | null)[] = [];
    L.subscribeListening(() => seen.push(L.getListening()?.phase ?? null));
    const one = begin()!;
    expect(L.getListening()).toMatchObject({ room: 'xqc', phase: 'listening', attempt: 1, attempts: 2, local: true });
    expect(begin()).toBeNull();
    one.update({ phase: 'finding' });
    one.end();
    expect(seen).toEqual(['listening', 'finding', null]);
    expect(begin()).not.toBeNull();
  });

  it('cancels only while listening; the strip goes at once, the window stays busy until the capture settles', () => {
    const one = begin()!;
    L.cancelListening();
    expect(one.signal.aborted).toBe(true);
    expect(L.getListening()).toBeNull();
    expect(L.isListening()).toBe(true);
    expect(begin()).toBeNull();
    one.end();
    expect(L.isListening()).toBe(false);

    const two = begin()!;
    two.update({ phase: 'finding' });
    L.cancelListening();
    expect(two.signal.aborted).toBe(false);
    expect(L.getListening()?.phase).toBe('finding');
  });

  it('sends the level at most every 33 ms, and never after a cancel', () => {
    const levels: number[] = [];
    L.subscribeLevel((v) => levels.push(v));
    const one = begin()!;
    one.level(0.1);
    vi.advanceTimersByTime(10);
    one.level(0.2);
    vi.advanceTimersByTime(30);
    one.level(0.3);
    vi.advanceTimersByTime(33);
    one.level(0.4);
    expect(levels).toEqual([0.1, 0.3, 0.4]);
    L.cancelListening();
    vi.advanceTimersByTime(100);
    one.level(0.5);
    expect(levels).toEqual([0.1, 0.3, 0.4]);
  });

  it('mirrors its state and level to the window it listens for', () => {
    const one = begin({ window: 'multichat-1', id: 'r1' })!;
    expect(one.id).toBe('r1');
    expect(sent('song-identify-state')[0]).toEqual([
      'multichat-1',
      'song-identify-state',
      { id: 'r1', phase: 'listening', attempt: 1, attempts: 2, captureMs: 5000, startedAt: null },
    ]);
    one.update({ startedAt: 1_000_500 });
    expect(sent('song-identify-state')[1][2]).toMatchObject({ startedAt: 1_000_500 });
    one.level(0.1);
    vi.advanceTimersByTime(40);
    one.level(0.2);
    vi.advanceTimersByTime(30);
    one.level(0.3);
    expect(sent('song-identify-level').map((c) => (c[2] as { level: number }).level)).toEqual([0.1, 0.3]);
  });
});

describe('asking the main window', () => {
  async function ask(room = 'XQC') {
    h.label = 'multichat-1';
    await L.requestRemoteIdentify(room);
    const [, , payload] = sent('song-identify-request')[0];
    return (payload as { id: string }).id;
  }

  it('asks only where no player is and the window is not the main one', () => {
    h.label = 'multichat-1';
    expect(L.listensRemotely()).toBe(true);
    h.video = {};
    expect(L.listensRemotely()).toBe(false);
    h.video = null;
    h.label = 'main';
    expect(L.listensRemotely()).toBe(false);
  });

  it("listens first, then asks, shows the main window's listening and posts its answer", async () => {
    const id = await ask();
    expect(sent('song-identify-request')[0]).toEqual(['main', 'song-identify-request', { id, room: 'XQC', window: 'multichat-1' }]);
    expect(L.getListening()).toMatchObject({ room: 'xqc', local: false, remote: { window: 'main', id } });

    fire('song-identify-state', { id, phase: 'listening', attempt: 1, attempts: 1, captureMs: 5000, startedAt: 1_000_100 });
    expect(L.getListening()).toMatchObject({ captureMs: 5000, startedAt: 1_000_100 });
    const levels: number[] = [];
    L.subscribeLevel((v) => levels.push(v));
    fire('song-identify-level', { id, level: 0.4 });
    fire('song-identify-level', { id: 'someone-else', level: 0.9 });
    expect(levels).toEqual([0.4]);

    const result = { status: 'no-match' };
    fire('song-identify-result', { id, result });
    await vi.dynamicImportSettled();
    expect(L.getListening()).toBeNull();
    expect(h.announce).toHaveBeenCalledWith(result, 'XQC');
    expect(listeners('song-identify-result')).toBe(0);
  });

  it('calls the request off and says no stream is playing when nobody answers in 3 s', async () => {
    const id = await ask();
    await vi.advanceTimersByTimeAsync(2999);
    expect(L.getListening()).not.toBeNull();
    await vi.advanceTimersByTimeAsync(1);
    await vi.dynamicImportSettled();
    expect(sent('song-identify-cancel')).toEqual([['main', 'song-identify-cancel', { id }]]);
    expect(h.announce).toHaveBeenCalledWith({ status: 'error', message: 'No stream is playing.' }, 'XQC');
    expect(L.getListening()).toBeNull();
  });

  it('lets the listening go quietly when its answer never comes', async () => {
    const id = await ask();
    // Two attempts of 5 s each, and each one's finding step (22 s): 54 s from the request.
    expect(L.FINDING_MS).toBe(22_000);
    fire('song-identify-state', { id, phase: 'listening', attempt: 1, attempts: 2, captureMs: 5000, startedAt: null });
    await vi.advanceTimersByTimeAsync(53_999);
    expect(L.getListening()).not.toBeNull();
    await vi.advanceTimersByTimeAsync(1);
    await vi.dynamicImportSettled();
    expect(L.getListening()).toBeNull();
    expect(h.announce).not.toHaveBeenCalled();
    expect(sent('song-identify-cancel')).toEqual([]);
  });

  it('gives the finding step its full time from when the clip is sent', async () => {
    const id = await ask();
    const state = (phase: string, attempt = 1) =>
      fire('song-identify-state', { id, phase, attempt, attempts: 1, captureMs: 5000, startedAt: null });
    state('listening');
    // The listening ran long: the clip goes 10 s in, so the answer may take
    // until 32 s, past the listening's own 27 s.
    await vi.advanceTimersByTimeAsync(10_000);
    state('finding');
    await vi.advanceTimersByTimeAsync(21_999);
    expect(L.getListening()?.phase).toBe('finding');
    const result = { status: 'match', song: { title: 'One', artist: 'A' } };
    fire('song-identify-result', { id, result });
    await vi.dynamicImportSettled();
    expect(h.announce).toHaveBeenCalledWith(result, 'XQC');
  });

  it('lets it go 22 s after the clip is sent when no answer comes', async () => {
    const id = await ask();
    const state = (phase: string) =>
      fire('song-identify-state', { id, phase, attempt: 1, attempts: 1, captureMs: 5000, startedAt: null });
    state('listening');
    await vi.advanceTimersByTimeAsync(10_000);
    state('finding');
    // A repeat of the same step does not move it again.
    await vi.advanceTimersByTimeAsync(5_000);
    state('finding');
    await vi.advanceTimersByTimeAsync(16_999);
    expect(L.getListening()).not.toBeNull();
    await vi.advanceTimersByTimeAsync(1);
    await vi.dynamicImportSettled();
    expect(L.getListening()).toBeNull();
    expect(h.announce).not.toHaveBeenCalled();
  });

  it('cancels through the main window, and posts nothing', async () => {
    const id = await ask();
    L.cancelListening();
    await vi.dynamicImportSettled();
    expect(sent('song-identify-cancel')).toEqual([['main', 'song-identify-cancel', { id }]]);
    expect(L.getListening()).toBeNull();
    expect(h.announce).not.toHaveBeenCalled();
    // A late answer goes nowhere.
    fire('song-identify-result', { id, result: { status: 'canceled' } });
    expect(h.announce).not.toHaveBeenCalled();
  });

  it('asks nothing more while one is out', async () => {
    await ask();
    await L.requestRemoteIdentify('XQC');
    expect(sent('song-identify-request')).toHaveLength(1);
  });
});

describe('the main window', () => {
  it('listens for other windows once, only in the main window', () => {
    h.label = 'multichat-1';
    L.installSongIdentifyHost();
    expect(listeners('song-identify-request')).toBe(0);
  });

  it("listens for the asking window and sends it the answer, posting nothing here", async () => {
    L.installSongIdentifyHost();
    L.installSongIdentifyHost();
    await vi.advanceTimersByTimeAsync(0);
    expect(listeners('song-identify-request')).toBe(1);
    const result = { status: 'no-match' };
    h.recognize.mockResolvedValue(result);
    fire('song-identify-request', { id: 'r1', room: 'XQC', window: 'multichat-1' });
    await vi.dynamicImportSettled();
    await vi.advanceTimersByTimeAsync(0);
    expect(h.recognize).toHaveBeenCalledWith(null, { room: null, remote: { window: 'multichat-1', id: 'r1' } });
    expect(sent('song-identify-result')).toEqual([['multichat-1', 'song-identify-result', { id: 'r1', result }]]);
    expect(h.announce).not.toHaveBeenCalled();
  });

  it('answers busy at once while it listens', async () => {
    L.installSongIdentifyHost();
    await vi.advanceTimersByTimeAsync(0);
    begin();
    fire('song-identify-request', { id: 'r2', room: null, window: 'overlay-1' });
    await vi.advanceTimersByTimeAsync(0);
    expect(h.recognize).not.toHaveBeenCalled();
    expect(sent('song-identify-result')).toEqual([['overlay-1', 'song-identify-result', { id: 'r2', result: { status: 'busy' } }]]);
  });

  it('cancels the listening it does for a window, and never starts one called off', async () => {
    L.installSongIdentifyHost();
    await vi.advanceTimersByTimeAsync(0);
    const one = begin({ window: 'multichat-1', id: 'r3' })!;
    fire('song-identify-cancel', { id: 'r3' });
    expect(one.signal.aborted).toBe(true);
    one.end();

    fire('song-identify-cancel', { id: 'r4' });
    fire('song-identify-request', { id: 'r4', room: null, window: 'multichat-1' });
    await vi.dynamicImportSettled();
    await vi.advanceTimersByTimeAsync(0);
    expect(h.recognize).not.toHaveBeenCalled();
  });
});
