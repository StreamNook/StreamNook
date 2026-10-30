// recognizeNowPlaying drives the window's one listening: a second while one
// runs is busy and says nothing; Cancel ends it as canceled with nothing sent
// to Rust; the window stays busy until the capture has really settled; any
// way out clears it; and a player rebuilt or paused under it cancels it.
import { beforeEach, describe, expect, it, vi } from 'vitest';

interface Capture {
  signal?: AbortSignal;
  onLevel?: (v: number) => void;
  onStart?: () => void;
  settle: (pcm: Int16Array | null) => void;
}
const h = vi.hoisted(() => ({
  captures: [] as Capture[],
  invoke: vi.fn(),
  /** The capture keeps going a moment after an abort (it is disconnecting). */
  holdOnAbort: false,
}));

vi.mock('./audioBoost', () => ({
  AUDIO_GRAPH_SUPPORTED: true,
  AUDIO_GRAPH_REFUSAL: '',
  // Settles when the test says, or with nothing once aborted (as the real one does).
  captureStreamSamples: (_v: unknown, _s: number, opts: Omit<Capture, 'settle'> = {}) =>
    new Promise<Int16Array | null>((resolve) => {
      h.captures.push({ ...opts, settle: resolve });
      opts.signal?.addEventListener('abort', () => {
        if (!h.holdOnAbort) resolve(null);
      });
    }),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => h.invoke(...a) }));
vi.mock('../stores/AppStore', () => ({
  useAppStore: { getState: () => ({ settings: {}, currentStream: null }) },
  mainChatRoom: () => null,
}));
vi.mock('../stores/chatConnectionStore', () => ({
  injectSystemMessage: vi.fn(),
  systemSourceFor: () => undefined,
  useChatConnectionStore: { getState: () => ({ channels: new Map() }) },
}));

import { recognizeNowPlaying } from './songId';
import { cancelListening, getListening, isListening } from './songListening';

type Listener = () => void;
function player() {
  const on = new Map<string, Set<Listener>>();
  return {
    paused: false,
    muted: false,
    readyState: 4,
    currentTime: 0,
    addEventListener: (t: string, f: Listener) => on.set(t, (on.get(t) ?? new Set()).add(f)),
    removeEventListener: (t: string, f: Listener) => on.get(t)?.delete(f),
    fire: (t: string) => on.get(t)?.forEach((f) => f()),
    count: () => [...on.values()].reduce((n, s) => n + s.size, 0),
  };
}
const asVideo = (p: ReturnType<typeof player>) => p as unknown as HTMLVideoElement;
const tick = () => new Promise<void>((r) => setTimeout(r, 0));
const pcm = () => new Int16Array(20000);

beforeEach(() => {
  h.captures.length = 0;
  h.invoke.mockReset();
});

describe('the listening recognizeNowPlaying drives', () => {
  it('shows its room and progress, and clears once it answers', async () => {
    h.invoke.mockResolvedValue({ title: 'One', artist: 'A' });
    const p = player();
    const run = recognizeNowPlaying(asVideo(p), { room: 'Kick:Chan' });
    expect(getListening()).toMatchObject({ room: 'kick:chan', phase: 'listening', startedAt: null, local: true });
    h.captures[0].onStart?.();
    expect(getListening()?.startedAt).toEqual(expect.any(Number));
    h.captures[0].settle(pcm());
    await tick();
    expect((await run).status).toBe('match');
    expect(getListening()).toBeNull();
    expect(isListening()).toBe(false);
    expect(p.count()).toBe(0);
  });

  it('answers busy to a second, which says nothing and starts nothing', async () => {
    h.invoke.mockResolvedValue(null);
    const p = player();
    const first = recognizeNowPlaying(asVideo(p), { room: 'xqc' });
    expect(await recognizeNowPlaying(asVideo(player()))).toEqual({ status: 'busy' });
    expect(h.captures).toHaveLength(1);
    cancelListening();
    expect((await first).status).toBe('canceled');
  });

  it('cancels as canceled with nothing sent, and stays busy until the capture settles', async () => {
    h.holdOnAbort = true;
    try {
      const run = recognizeNowPlaying(asVideo(player()), { room: 'xqc' });
      const cap = h.captures[0];
      cancelListening();
      expect(cap.signal?.aborted).toBe(true);
      // The strip goes at once; the capture has not settled, so no new
      // listening may overlap it.
      expect(getListening()).toBeNull();
      expect(isListening()).toBe(true);
      expect(await recognizeNowPlaying(asVideo(player()))).toEqual({ status: 'busy' });
      expect(h.captures).toHaveLength(1);
      // Whatever it heard is dropped.
      cap.settle(pcm());
      expect((await run).status).toBe('canceled');
      expect(h.invoke).not.toHaveBeenCalled();
      expect(isListening()).toBe(false);
    } finally {
      h.holdOnAbort = false;
    }
  });

  it('clears on an error from Rust', async () => {
    h.invoke.mockRejectedValue(new Error('rate limited'));
    const run = recognizeNowPlaying(asVideo(player()), { room: 'xqc' });
    h.captures[0].settle(pcm());
    expect(getListening()?.phase).toBe('listening');
    await tick();
    expect(await run).toEqual({ status: 'error', message: 'rate limited' });
    expect(getListening()).toBeNull();
    expect(isListening()).toBe(false);
  });

  it('is finding once the clip is sent, when Cancel no longer acts', async () => {
    let answer!: (v: unknown) => void;
    h.invoke.mockReturnValue(new Promise((r) => (answer = r)));
    const run = recognizeNowPlaying(asVideo(player()), { room: 'xqc' });
    h.captures[0].settle(pcm());
    await tick();
    expect(getListening()?.phase).toBe('finding');
    cancelListening();
    expect(getListening()?.phase).toBe('finding');
    answer({ title: 'Two', artist: 'B' });
    expect((await run).status).toBe('match');
  });

  it('cancels when the player is rebuilt or paused under it', async () => {
    for (const event of ['emptied', 'pause']) {
      const p = player();
      const run = recognizeNowPlaying(asVideo(p), { room: 'xqc' });
      p.fire(event);
      expect((await run).status).toBe('canceled');
      expect(h.invoke).not.toHaveBeenCalled();
      expect(p.count()).toBe(0);
    }
  });
});
