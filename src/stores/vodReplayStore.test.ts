import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args: unknown) => invoke(cmd, args) }));
vi.mock('./chatConnectionStore', () => ({ EMPTY_ROOM_STATE: {} }));
vi.mock('../keybindings/playerControls', () => ({ getPlayerControls: () => null }));
vi.mock('../utils/logger', () => ({ Logger: { warn: () => {} } }));

import {
  beginVodReplay,
  registerReplayClock,
  setVodReplayShowing,
  stopVodReplay,
  useVodReplayStore,
} from './vodReplayStore';

const row = (id: string) => ({ id }) as never;
const step = (rows: string[], extra: Partial<{ reset: boolean; loading: boolean; error: string | null }> = {}) => ({
  reset: false,
  rows: rows.map(row),
  loading: false,
  error: null,
  ...extra,
});
const shown = () => useVodReplayStore.getState().messages.map((m) => (m as { id: string }).id);
const calls = (cmd: string) => invoke.mock.calls.filter(([c]) => c === cmd);

let position = 100;
let unregister: () => void = () => {};

beforeEach(() => {
  vi.useFakeTimers();
  invoke.mockReset();
  position = 100;
  unregister = registerReplayClock(() => position);
});

afterEach(() => {
  stopVodReplay();
  unregister();
  vi.useRealTimers();
});

describe('VOD chat replay through Rust', () => {
  it('starts Rust at the playhead and adds the rows each report returns', async () => {
    const answers = [step([], { reset: true }), step(['a', 'b']), step(['c'])];
    invoke.mockImplementation(async (cmd: string) => (cmd === 'vod_replay_tick' ? answers.shift() ?? step([]) : undefined));

    beginVodReplay('v1', 'Chan');
    await vi.advanceTimersByTimeAsync(0);
    expect(calls('vod_replay_begin')[0][1]).toEqual({
      videoId: 'v1',
      login: 'chan',
      position: 100,
      startOffset: 0,
      endOffset: null,
    });
    await vi.advanceTimersByTimeAsync(750);
    expect(shown()).toEqual(['a', 'b']);
    await vi.advanceTimersByTimeAsync(750);
    expect(shown()).toEqual(['a', 'b', 'c']);
    expect(calls('vod_replay_tick').at(-1)?.[1]).toEqual({ videoId: 'v1', position: 100 });
  });

  it('replaces the rows on a reset and reports again soon after a seek it cannot answer yet', async () => {
    const answers = [step(['a', 'b'], { reset: true }), step([], { reset: true, loading: true }), step(['z'])];
    invoke.mockImplementation(async (cmd: string) => (cmd === 'vod_replay_tick' ? answers.shift() ?? step([]) : undefined));

    beginVodReplay('v1', 'chan');
    await vi.advanceTimersByTimeAsync(0);
    expect(shown()).toEqual(['a', 'b']);
    position = 2_000;
    await vi.advanceTimersByTimeAsync(750);
    expect(shown()).toEqual([]);
    const ticks = calls('vod_replay_tick').length;
    await vi.advanceTimersByTimeAsync(400);
    expect(calls('vod_replay_tick').length).toBe(ticks + 1);
    expect(shown()).toEqual(['z']);
  });

  it('drops an answer that arrives after the session ended, and stops after the begin it follows', async () => {
    let release: (v: unknown) => void = () => {};
    invoke.mockImplementation((cmd: string) =>
      cmd === 'vod_replay_tick' ? new Promise((r) => (release = r)) : Promise.resolve(),
    );

    beginVodReplay('v1', 'chan');
    await vi.advanceTimersByTimeAsync(0);
    stopVodReplay();
    release(step(['late'], { reset: true }));
    await vi.advanceTimersByTimeAsync(0);
    expect(shown()).toEqual([]);
    expect(useVodReplayStore.getState().active).toBe(false);
    const order = invoke.mock.calls.map(([c]) => c);
    expect(order.indexOf('vod_replay_stop')).toBeGreaterThan(order.indexOf('vod_replay_begin'));
  });

  it('sends a clip its window, and a new session starts on VOD Chat', async () => {
    invoke.mockResolvedValue(step([]));
    beginVodReplay('v1', 'chan');
    setVodReplayShowing('live');
    expect(useVodReplayStore.getState().showing).toBe('live');
    position = 0;
    beginVodReplay('v2', 'chan', { startOffset: 2_781, endOffset: 2_811 });
    await vi.advanceTimersByTimeAsync(0);
    expect(useVodReplayStore.getState().showing).toBe('replay');
    expect(calls('vod_replay_begin').at(-1)?.[1]).toMatchObject({ videoId: 'v2', startOffset: 2_781, endOffset: 2_811, position: 0 });
  });
});
