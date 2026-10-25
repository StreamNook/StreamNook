import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// How restartStream treats overlapping and stale asks: one restart runs at a
// time, and a restart for a stream that has since been replaced does nothing.

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(),
}));

vi.mock('../bootPreload', () => ({ takePreloadedSettings: () => null }));

import { useAppStore } from './AppStore';

const stream = { user_login: 'chan', user_name: 'Chan', user_id: '1' };

const calls = (cmd: string) => invokeMock.mock.calls.filter(([c]) => c === cmd);

/** Rust answers a start with a fresh relay URL. */
function answerStarts() {
  let n = 0;
  invokeMock.mockImplementation(async (cmd: string) =>
    cmd === 'start_stream' ? { url: `http://localhost/relay-${++n}.m3u8`, quality: 'best' } : undefined,
  );
}

beforeEach(() => {
  vi.useFakeTimers();
  invokeMock.mockReset();
  answerStarts();
  useAppStore.setState({
    currentStream: stream as never,
    currentMediaType: 'live',
    settings: { quality: 'best' } as never,
    streamUrl: 'http://localhost/old.m3u8',
    isAutoSwitching: false,
    isRestartingStream: false,
    addToast: vi.fn() as never,
  });
});

afterEach(() => vi.useRealTimers());

describe('restartStream', () => {
  it('runs one restart for two overlapping calls', async () => {
    const first = useAppStore.getState().restartStream({ quiet: true });
    const second = useAppStore.getState().restartStream({ quiet: true });
    await vi.advanceTimersByTimeAsync(1000);
    await Promise.all([first, second]);
    expect(calls('stop_stream')).toHaveLength(1);
    expect(calls('start_stream')).toHaveLength(1);
    expect(useAppStore.getState().isRestartingStream).toBe(false);
  });

  it('runs a new restart once the earlier one has ended', async () => {
    const first = useAppStore.getState().restartStream({ quiet: true });
    await vi.advanceTimersByTimeAsync(1000);
    await first;
    const second = useAppStore.getState().restartStream({ quiet: true });
    await vi.advanceTimersByTimeAsync(1000);
    await second;
    expect(calls('start_stream')).toHaveLength(2);
  });

  it('does nothing when the stream is no longer the one the caller saw', async () => {
    const restart = useAppStore.getState().restartStream({ quiet: true, ifUrl: 'http://localhost/old.m3u8' });
    // A swap lands while the restart waits for the swap in flight to settle.
    useAppStore.setState({ streamUrl: 'http://localhost/swapped.m3u8' });
    await vi.advanceTimersByTimeAsync(1000);
    await restart;
    expect(calls('stop_stream')).toHaveLength(0);
    expect(calls('start_stream')).toHaveLength(0);
    expect(useAppStore.getState().streamUrl).toBe('http://localhost/swapped.m3u8');
    expect(useAppStore.getState().isRestartingStream).toBe(false);
  });

  it('restarts when the stream is still the one the caller saw', async () => {
    const restart = useAppStore.getState().restartStream({ quiet: true, ifUrl: 'http://localhost/old.m3u8' });
    await vi.advanceTimersByTimeAsync(1000);
    await restart;
    expect(calls('start_stream')).toHaveLength(1);
    expect(useAppStore.getState().streamUrl).toBe('http://localhost/relay-1.m3u8');
  });

  it('does not wait on itself when a failed restart ends in a restart', async () => {
    let starts = 0;
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'start_stream') {
        starts++;
        if (starts === 1) throw new Error('usher 404');
        return { url: 'http://localhost/relay-2.m3u8', quality: 'best' };
      }
      if (cmd === 'check_stream_online') return null;
      if (cmd === 'watch_session_resolve_offline') return { action: 'resume', why: 'live again' };
      return undefined;
    });
    const restart = useAppStore.getState().restartStream({ quiet: true });
    await vi.advanceTimersByTimeAsync(2000);
    await restart;
    expect(starts).toBe(2);
    expect(useAppStore.getState().streamUrl).toBe('http://localhost/relay-2.m3u8');
  });
});
