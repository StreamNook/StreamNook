import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('hls.js', () => ({
  default: { Events: { LEVEL_UPDATED: 'hlsLevelUpdated', FRAG_LOADED: 'hlsFragLoaded' } },
}));

const { RESYNC_WINDOW_MS, followLiveAfterResync } = await import('./liveResync');

/** Just enough of hls.js: listeners by event name and a settable sync point. */
function fakeHls() {
  const on = new Map<string, Set<() => void>>();
  return {
    liveSyncPosition: null as number | null,
    on(e: string, f: () => void) {
      if (!on.has(e)) on.set(e, new Set());
      on.get(e)!.add(f);
    },
    off(e: string, f: () => void) {
      on.get(e)?.delete(f);
    },
    fire(e: string) {
      for (const f of [...(on.get(e) ?? [])]) f();
    },
    count(e: string) {
      return on.get(e)?.size ?? 0;
    },
  };
}

describe('followLiveAfterResync', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const start = () => {
    const hls = fakeHls();
    const video = { currentTime: 10 };
    const onStuck = vi.fn();
    const stop = followLiveAfterResync(hls as never, video as never, onStuck);
    return { hls, video, onStuck, stop };
  };

  it('moves the playhead to each reload that lands ahead of it', () => {
    const { hls, video } = start();
    hls.liveSyncPosition = 40; // the parked relay's held edge
    hls.fire('hlsLevelUpdated');
    expect(video.currentTime).toBe(40);
    hls.liveSyncPosition = 70; // the rebuilt edge
    hls.fire('hlsLevelUpdated');
    expect(video.currentTime).toBe(70);
    hls.liveSyncPosition = 70.5; // not worth a seek
    hls.fire('hlsLevelUpdated');
    expect(video.currentTime).toBe(70);
  });

  it('reports a resync that never loads a fragment', () => {
    const { hls, onStuck } = start();
    vi.advanceTimersByTime(RESYNC_WINDOW_MS);
    expect(onStuck).toHaveBeenCalledTimes(1);
    expect(hls.count('hlsLevelUpdated')).toBe(0);
  });

  it('a loaded fragment means it is not stuck', () => {
    const { hls, onStuck } = start();
    hls.fire('hlsFragLoaded');
    vi.advanceTimersByTime(RESYNC_WINDOW_MS);
    expect(onStuck).not.toHaveBeenCalled();
  });

  it('cancel stops following and never reports', () => {
    const { hls, video, onStuck, stop } = start();
    stop();
    hls.liveSyncPosition = 99;
    hls.fire('hlsLevelUpdated');
    vi.advanceTimersByTime(RESYNC_WINDOW_MS);
    expect(video.currentTime).toBe(10);
    expect(onStuck).not.toHaveBeenCalled();
    expect(hls.count('hlsFragLoaded')).toBe(0);
  });
});
