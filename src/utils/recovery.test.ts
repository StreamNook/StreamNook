import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('hls.js', () => ({
  default: { Events: { LEVEL_UPDATED: 'hlsLevelUpdated', FRAG_BUFFERED: 'hlsFragBuffered' } },
}));

const { RESYNC_WINDOW_MS, followLiveAfterResync } = await import('./liveResync');
const { watchStalls } = await import('./stallWatchdog');
const { createRecovery } = await import('./recovery');

/** Just enough of hls.js for the resume check: listeners by event name and the
 *  start of the window the origin serves. */
function fakeHls() {
  type Listener = (e?: string, data?: unknown) => void;
  const on = new Map<string, Set<Listener>>();
  return {
    liveSyncPosition: null as number | null,
    latestLevelDetails: { fragmentStart: 0 } as { fragmentStart: number } | null,
    on(e: string, f: Listener) {
      if (!on.has(e)) on.set(e, new Set());
      on.get(e)!.add(f);
    },
    off(e: string, f: Listener) {
      on.get(e)?.delete(f);
    },
  };
}

/** A media element whose playhead stays pinned unless a test moves it. */
function fakeVideo() {
  const listeners = new Map<string, Set<() => void>>();
  return {
    paused: false,
    currentTime: 10,
    buffered: { length: 0, start: () => 0, end: () => 0 },
    addEventListener(e: string, f: () => void) {
      if (!listeners.has(e)) listeners.set(e, new Set());
      listeners.get(e)!.add(f);
    },
    removeEventListener(e: string, f: () => void) {
      listeners.get(e)?.delete(f);
    },
  };
}

describe('createRecovery', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  /** The player's shape: the resume check and the freeze watchdog both end in
   *  one guard, whose stop releases both checks. */
  const wired = (opts: { current?: () => boolean; refused?: () => boolean } = {}) => {
    const hls = fakeHls();
    const video = fakeVideo();
    const restart = vi.fn();
    const stop = vi.fn();
    const onFrozen = vi.fn();
    let stopResync: () => void = () => {};
    let stopWatch: () => void = () => {};
    const recover = createRecovery({
      current: opts.current ?? (() => true),
      refused: opts.refused ?? (() => false),
      stop: () => {
        stop();
        stopResync();
        stopWatch();
      },
      restart: (why: string) => restart(why),
    });
    stopResync = followLiveAfterResync(hls as never, video as never, () => recover('resume'));
    stopWatch = watchStalls(video as never, {
      onFrozen: () => {
        onFrozen();
        recover('frozen');
      },
    });
    return { restart, stop, onFrozen };
  };

  it('restarts once when the resume check and the freeze watchdog fire on the same tick', () => {
    const { restart, stop } = wired();
    vi.advanceTimersByTime(RESYNC_WINDOW_MS);
    expect(restart).toHaveBeenCalledTimes(1);
    expect(stop).toHaveBeenCalledTimes(1);
  });

  it('restarts once and stops once over 60 seconds of a pinned playhead', () => {
    const { restart, stop } = wired();
    vi.advanceTimersByTime(60_000);
    expect(restart).toHaveBeenCalledTimes(1);
    expect(stop).toHaveBeenCalledTimes(1);
  });

  it('stops the freeze watchdog with the restart, so it never fires again', () => {
    const video = fakeVideo();
    const restart = vi.fn();
    const onFrozen = vi.fn();
    let stopWatch: () => void = () => {};
    const recover = createRecovery({
      current: () => true,
      refused: () => false,
      stop: () => stopWatch(),
      restart,
    });
    stopWatch = watchStalls(video as never, {
      onFrozen: () => {
        onFrozen();
        recover();
      },
    });
    vi.advanceTimersByTime(60_000);
    expect(onFrozen).toHaveBeenCalledTimes(1);
    expect(restart).toHaveBeenCalledTimes(1);
  });

  it('runs nothing while refused, then restarts once when it is allowed', () => {
    let refused = true;
    const { restart, stop } = wired({ refused: () => refused });
    vi.advanceTimersByTime(RESYNC_WINDOW_MS);
    expect(restart).not.toHaveBeenCalled();
    expect(stop).not.toHaveBeenCalled();
    refused = false;
    vi.advanceTimersByTime(60_000);
    expect(restart).toHaveBeenCalledTimes(1);
    expect(stop).toHaveBeenCalledTimes(1);
  });

  it('runs nothing for a player that is no longer current', () => {
    const { restart, stop } = wired({ current: () => false });
    vi.advanceTimersByTime(60_000);
    expect(restart).not.toHaveBeenCalled();
    expect(stop).not.toHaveBeenCalled();
  });

  it('passes its arguments to the restart and says whether it ran', () => {
    const restart = vi.fn();
    const recover = createRecovery({
      current: () => true,
      refused: () => false,
      stop: () => {},
      restart: (why: string, quiet = true) => restart(why, quiet),
    });
    expect(recover('first', false)).toBe(true);
    expect(recover('second')).toBe(false);
    expect(restart).toHaveBeenCalledTimes(1);
    expect(restart).toHaveBeenCalledWith('first', false);
  });
});
