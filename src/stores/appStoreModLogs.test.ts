import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// The mod-log history of a channel is loaded once per open. A load that fails
// (Rust could not read the saved history) retries on its own, after 1, 2, 4, 8,
// 16 and then 30 seconds, until it succeeds or the channel closes.

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(),
}));

vi.mock('../bootPreload', () => ({ takePreloadedSettings: () => null }));

import { useAppStore } from './AppStore';

const entry = (id: string) => ({ id, channel: 'chan', action: 'ban', timestamp: '2026-10-24T00:00:00Z' });

const REJECTED = 'Failed to read mod logs file';

/** How many times the history was asked for (the logger also calls invoke). */
const loadCalls = () => invokeMock.mock.calls.filter(([command]) => command === 'load_mod_logs').length;

/** Answer each load_mod_logs call with the next of `answers`, the last one
 *  repeating: a string rejects with it, a list resolves with it. */
function answerLoads(...answers: Array<string | ReturnType<typeof entry>[]>) {
  let call = 0;
  invokeMock.mockImplementation(async (command: string) => {
    if (command !== 'load_mod_logs') return undefined;
    const answer = answers[Math.min(call++, answers.length - 1)];
    if (typeof answer === 'string') throw answer;
    return answer;
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  invokeMock.mockReset();
  useAppStore.setState({ modLogs: [], loadedModLogChannels: new Set<string>() });
});

afterEach(() => {
  useAppStore.setState({ loadedModLogChannels: new Set<string>() });
  vi.useRealTimers();
});

describe('loadModLogsForChannel', () => {
  it('retries a rejected load on its own until it succeeds', async () => {
    answerLoads(REJECTED, REJECTED, [entry('a1')]);
    const loading = useAppStore.getState().loadModLogsForChannel('Chan');

    await vi.advanceTimersByTimeAsync(0);
    expect(loadCalls()).toBe(1);
    // The channel stays marked loaded while the retries run, so a re-render does not start another load.
    expect(useAppStore.getState().loadedModLogChannels.has('chan')).toBe(true);
    await vi.advanceTimersByTimeAsync(1000);
    expect(loadCalls()).toBe(2);
    await vi.advanceTimersByTimeAsync(2000);
    await loading;

    expect(loadCalls()).toBe(3);
    expect(useAppStore.getState().loadedModLogChannels.has('chan')).toBe(true);
    expect(useAppStore.getState().modLogs.map((l) => l.id)).toEqual(['a1']);
    // A load that succeeded is not repeated.
    await vi.advanceTimersByTimeAsync(120_000);
    expect(loadCalls()).toBe(3);
  });

  it('waits 1, 2, 4, 8, 16, 30 and then 30 seconds between retries', async () => {
    answerLoads(REJECTED);
    const loading = useAppStore.getState().loadModLogsForChannel('chan');

    await vi.advanceTimersByTimeAsync(0);
    let calls = 1;
    expect(loadCalls()).toBe(calls);
    for (const seconds of [1, 2, 4, 8, 16, 30, 30]) {
      await vi.advanceTimersByTimeAsync(seconds * 1000 - 1);
      expect(loadCalls()).toBe(calls);
      await vi.advanceTimersByTimeAsync(1);
      calls += 1;
      expect(loadCalls()).toBe(calls);
    }

    useAppStore.getState().pruneModLogsToChannels([]);
    await vi.advanceTimersByTimeAsync(60_000);
    await loading;
    expect(loadCalls()).toBe(calls);
  });

  it('stops retrying when the channel closes', async () => {
    answerLoads(REJECTED);
    const loading = useAppStore.getState().loadModLogsForChannel('chan');
    await vi.advanceTimersByTimeAsync(0);
    expect(loadCalls()).toBe(1);

    useAppStore.getState().pruneModLogsToChannels([]);
    await vi.advanceTimersByTimeAsync(120_000);
    await loading;
    expect(loadCalls()).toBe(1);
    expect(useAppStore.getState().loadedModLogChannels.has('chan')).toBe(false);
  });

  it('retries a reopened channel from its newest load only', async () => {
    answerLoads(REJECTED);
    const first = useAppStore.getState().loadModLogsForChannel('chan');
    await vi.advanceTimersByTimeAsync(0);
    useAppStore.getState().pruneModLogsToChannels([]);
    const second = useAppStore.getState().loadModLogsForChannel('chan');
    await vi.advanceTimersByTimeAsync(0);
    expect(loadCalls()).toBe(2);

    // Both waits end at the same moment; only the newest load asks again.
    await vi.advanceTimersByTimeAsync(1000);
    expect(loadCalls()).toBe(3);

    useAppStore.getState().pruneModLogsToChannels([]);
    await vi.advanceTimersByTimeAsync(60_000);
    await Promise.all([first, second]);
    expect(loadCalls()).toBe(3);
  });

  it('loads a channel that loaded fine only once', async () => {
    answerLoads([entry('a1')]);
    await useAppStore.getState().loadModLogsForChannel('chan');
    await useAppStore.getState().loadModLogsForChannel('chan');
    expect(loadCalls()).toBe(1);
  });
});
