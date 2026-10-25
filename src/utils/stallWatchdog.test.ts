import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { watchStalls } from './stallWatchdog';

/** A media element with settable playhead and state, and counted listeners. */
function fakeVideo() {
  const listeners = new Map<string, Set<() => void>>();
  return {
    currentTime: 100,
    paused: false,
    seeking: false,
    readyState: 4,
    addEventListener(e: string, f: () => void) {
      if (!listeners.has(e)) listeners.set(e, new Set());
      listeners.get(e)!.add(f);
    },
    removeEventListener(e: string, f: () => void) {
      listeners.get(e)?.delete(f);
    },
    emit(e: string) {
      for (const f of [...(listeners.get(e) ?? [])]) f();
    },
    total() {
      let n = 0;
      for (const set of listeners.values()) n += set.size;
      return n;
    },
  };
}

describe('watchStalls', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const start = () => {
    const video = fakeVideo();
    const onFrozen = vi.fn();
    const stop = watchStalls(video as never, { onFrozen });
    return { video, onFrozen, stop };
  };

  it('fires once after a pinned playhead has stood for 8 seconds while playing', () => {
    const { onFrozen } = start();
    vi.advanceTimersByTime(7999);
    expect(onFrozen).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(onFrozen).toHaveBeenCalledTimes(1);
  });

  it('never fires while paused', () => {
    const { video, onFrozen } = start();
    video.paused = true;
    vi.advanceTimersByTime(60000);
    expect(onFrozen).not.toHaveBeenCalled();
  });

  it('never fires while seeking or before the element holds media', () => {
    const { video, onFrozen } = start();
    video.seeking = true;
    vi.advanceTimersByTime(30000);
    video.seeking = false;
    video.readyState = 0;
    vi.advanceTimersByTime(30000);
    expect(onFrozen).not.toHaveBeenCalled();
  });

  it('never fires while the playhead advances', () => {
    const { video, onFrozen } = start();
    for (let i = 0; i < 60; i++) {
      vi.advanceTimersByTime(1000);
      video.currentTime += 1;
      video.emit('timeupdate');
    }
    expect(onFrozen).not.toHaveBeenCalled();
  });

  it('counts again from a pause, a seek or a move of the playhead', () => {
    const { video, onFrozen } = start();
    vi.advanceTimersByTime(7000);
    video.emit('pause');
    vi.advanceTimersByTime(7000);
    expect(onFrozen).not.toHaveBeenCalled();
    video.emit('seeking');
    vi.advanceTimersByTime(7000);
    expect(onFrozen).not.toHaveBeenCalled();
    video.currentTime += 1;
    vi.advanceTimersByTime(7000);
    expect(onFrozen).not.toHaveBeenCalled();
    vi.advanceTimersByTime(2000); // the move registers at the next sample
    expect(onFrozen).toHaveBeenCalledTimes(1);
  });

  it('holds for the cooldown after a report, then reports a stream that is still frozen', () => {
    const { onFrozen } = start();
    vi.advanceTimersByTime(8000);
    expect(onFrozen).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(14000);
    expect(onFrozen).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(1000);
    expect(onFrozen).toHaveBeenCalledTimes(2);
  });

  it('stop releases the interval and every listener', () => {
    const { video, onFrozen, stop } = start();
    expect(video.total()).toBeGreaterThan(0);
    stop();
    expect(video.total()).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
    vi.advanceTimersByTime(60000);
    expect(onFrozen).not.toHaveBeenCalled();
  });
});
