import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('hls.js', () => ({
  default: { Events: { LEVEL_UPDATED: 'hlsLevelUpdated', FRAG_BUFFERED: 'hlsFragBuffered' } },
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

/** A media element with settable buffered ranges, time and paused state. */
function fakeVideo() {
  const listeners = new Map<string, Set<() => void>>();
  let ranges: Array<[number, number]> = [];
  const seeks: number[] = [];
  let time = 10;
  return {
    paused: false,
    get currentTime() {
      return time;
    },
    set currentTime(t: number) {
      seeks.push(t);
      time = t;
    },
    get buffered() {
      return {
        length: ranges.length,
        start: (i: number) => ranges[i][0],
        end: (i: number) => ranges[i][1],
      };
    },
    setBuffered(r: Array<[number, number]>) {
      ranges = r;
    },
    advance(t: number) {
      time = t;
    },
    seeks,
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
    count(e: string) {
      return listeners.get(e)?.size ?? 0;
    },
  };
}

describe('followLiveAfterResync', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const start = (seek = true) => {
    const hls = fakeHls();
    const video = fakeVideo();
    const onStuck = vi.fn();
    const stop = followLiveAfterResync(hls as never, video as never, onStuck, undefined, { seek });
    return { hls, video, onStuck, stop };
  };

  it('does not seek while the place it is going to is not buffered', () => {
    const { hls, video } = start();
    hls.liveSyncPosition = 40;
    video.setBuffered([[8, 12]]); // the live edge is not downloaded yet
    hls.fire('hlsLevelUpdated');
    hls.fire('hlsFragBuffered');
    expect(video.seeks).toEqual([]);
  });

  it('does not seek into a hole between buffered ranges', () => {
    const { hls, video } = start();
    hls.liveSyncPosition = 30;
    video.setBuffered([[8, 12], [40, 50]]);
    hls.fire('hlsLevelUpdated');
    expect(video.seeks).toEqual([]);
  });

  it('seeks once to the lower of the sync point and the buffered end less the cushion', () => {
    const { hls, video } = start();
    hls.liveSyncPosition = 40;
    video.setBuffered([[8, 12]]);
    hls.fire('hlsLevelUpdated');
    expect(video.seeks).toEqual([]);
    video.setBuffered([[8, 12], [30, 36]]);
    hls.fire('hlsFragBuffered');
    expect(video.seeks).toEqual([34]); // 36 - 2: inside the range, below the sync point
    hls.liveSyncPosition = 90;
    video.setBuffered([[30, 95]]);
    hls.fire('hlsLevelUpdated');
    hls.fire('hlsFragBuffered');
    expect(video.seeks).toEqual([34]); // one seek, then it stops following
    expect(hls.count('hlsLevelUpdated')).toBe(0);
    expect(hls.count('hlsFragBuffered')).toBe(0);
  });

  it('uses the sync point when the buffer reaches past it', () => {
    const { hls, video } = start();
    hls.liveSyncPosition = 20;
    video.setBuffered([[8, 60]]);
    hls.fire('hlsLevelUpdated');
    expect(video.seeks).toEqual([20]);
  });

  it('leaves a playhead that is already near the target', () => {
    const { hls, video } = start();
    hls.liveSyncPosition = 10.5;
    video.setBuffered([[8, 60]]);
    hls.fire('hlsLevelUpdated');
    expect(video.seeks).toEqual([]);
  });

  it('is stuck when the video never plays and its time does not advance', () => {
    const { hls, video, onStuck } = start();
    vi.advanceTimersByTime(RESYNC_WINDOW_MS);
    expect(onStuck).toHaveBeenCalledTimes(1);
    expect(hls.count('hlsLevelUpdated')).toBe(0);
    expect(video.count('playing')).toBe(0);
  });

  it('is not stuck once the video plays', () => {
    const { video, onStuck } = start();
    video.emit('playing');
    vi.advanceTimersByTime(RESYNC_WINDOW_MS);
    expect(onStuck).not.toHaveBeenCalled();
  });

  it('is not stuck when the time advances', () => {
    const { video, onStuck } = start();
    video.advance(12);
    vi.advanceTimersByTime(RESYNC_WINDOW_MS);
    expect(onStuck).not.toHaveBeenCalled();
  });

  it('measures advance from the playhead after its own seek', () => {
    const { hls, video, onStuck } = start();
    hls.liveSyncPosition = 20;
    video.setBuffered([[8, 60]]);
    hls.fire('hlsLevelUpdated');
    vi.advanceTimersByTime(RESYNC_WINDOW_MS);
    expect(onStuck).toHaveBeenCalledTimes(1);
  });

  it('never reports a paused video', () => {
    const { video, onStuck } = start();
    video.paused = true;
    vi.advanceTimersByTime(RESYNC_WINDOW_MS);
    expect(onStuck).not.toHaveBeenCalled();
  });

  it('leaves the move to the caller when told not to seek', () => {
    const { hls, video, onStuck } = start(false);
    hls.liveSyncPosition = 20;
    video.setBuffered([[8, 60]]);
    hls.fire('hlsLevelUpdated');
    expect(video.seeks).toEqual([]);
    vi.advanceTimersByTime(RESYNC_WINDOW_MS);
    expect(onStuck).toHaveBeenCalledTimes(1);
  });

  it('cancel stops following and never reports', () => {
    const { hls, video, onStuck, stop } = start();
    stop();
    hls.liveSyncPosition = 99;
    video.setBuffered([[8, 120]]);
    hls.fire('hlsLevelUpdated');
    vi.advanceTimersByTime(RESYNC_WINDOW_MS);
    expect(video.seeks).toEqual([]);
    expect(onStuck).not.toHaveBeenCalled();
    expect(video.count('playing')).toBe(0);
  });
});
