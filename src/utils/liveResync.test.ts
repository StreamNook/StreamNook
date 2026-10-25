import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('hls.js', () => ({
  default: { Events: { LEVEL_UPDATED: 'hlsLevelUpdated', FRAG_BUFFERED: 'hlsFragBuffered' } },
}));

const { RESYNC_WINDOW_MS, followLiveAfterResync } = await import('./liveResync');

/** Just enough of hls.js: listeners by event name, a settable sync point and
 *  the start of the window the origin serves. */
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
    fire(e: string, data?: unknown) {
      for (const f of [...(on.get(e) ?? [])]) f(e, data);
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

  const start = () => {
    const hls = fakeHls();
    const video = fakeVideo();
    const onStuck = vi.fn();
    const stop = followLiveAfterResync(hls as never, video as never, onStuck);
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
    expect(hls.count('hlsLevelUpdated')).toBe(1); // only the stuck check's reload tracker remains
    expect(hls.count('hlsFragBuffered')).toBe(1); // only the stuck check's fragment tracker remains
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
    expect(video.count('timeupdate')).toBe(0);
  });

  const liveFragment = { frag: { type: 'main', start: 5, duration: 2 } };

  it('is not stuck once the video plays on', () => {
    const { hls, video, onStuck } = start();
    hls.fire('hlsLevelUpdated');
    hls.fire('hlsFragBuffered', liveFragment);
    video.emit('playing');
    video.advance(14);
    video.emit('timeupdate');
    vi.advanceTimersByTime(RESYNC_WINDOW_MS * 2);
    expect(onStuck).not.toHaveBeenCalled();
  });

  it('is stuck one window after new live data when the video plays but its time stays pinned', () => {
    const { hls, video, onStuck } = start();
    hls.fire('hlsLevelUpdated');
    hls.fire('hlsFragBuffered', liveFragment);
    video.emit('playing');
    vi.advanceTimersByTime(RESYNC_WINDOW_MS - 1);
    expect(onStuck).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(onStuck).toHaveBeenCalledTimes(1);
  });

  it('reports a pinned playhead once, within four windows, under a seek every 1.5 seconds', () => {
    const { hls, video, onStuck } = start();
    hls.fire('hlsLevelUpdated');
    hls.fire('hlsFragBuffered', liveFragment);
    for (let elapsed = 0; elapsed < RESYNC_WINDOW_MS * 4; elapsed += 1500) {
      vi.advanceTimersByTime(1500);
      video.emit('seeking');
    }
    expect(onStuck).toHaveBeenCalledTimes(1);
  });

  it('does not count a seek past the re-arm cap as progress', () => {
    const { hls, video, onStuck } = start();
    hls.fire('hlsLevelUpdated');
    hls.fire('hlsFragBuffered', liveFragment);
    for (let i = 0; i < 3; i++) {
      vi.advanceTimersByTime(1000);
      video.emit('seeking');
    }
    vi.advanceTimersByTime(1000);
    video.advance(40); // a jump the element made on its own
    video.emit('seeking');
    vi.advanceTimersByTime(RESYNC_WINDOW_MS);
    expect(onStuck).toHaveBeenCalledTimes(1);
  });

  it('counts real playback after the re-arm cap as progress', () => {
    const { hls, video, onStuck } = start();
    hls.fire('hlsLevelUpdated');
    hls.fire('hlsFragBuffered', liveFragment);
    for (let i = 0; i < 3; i++) {
      vi.advanceTimersByTime(1000);
      video.emit('seeking');
    }
    vi.advanceTimersByTime(1000);
    video.advance(40);
    video.emit('seeking');
    video.advance(44);
    video.emit('timeupdate');
    vi.advanceTimersByTime(RESYNC_WINDOW_MS * 2);
    expect(onStuck).not.toHaveBeenCalled();
  });

  it('is stuck at the window when one seek lands at 5 seconds and no live data arrives', () => {
    const { video, onStuck } = start();
    vi.advanceTimersByTime(5000);
    video.advance(30);
    video.emit('seeking');
    vi.advanceTimersByTime(RESYNC_WINDOW_MS - 5000 - 1);
    expect(onStuck).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(onStuck).toHaveBeenCalledTimes(1);
  });

  it('is not stuck when the time advances', () => {
    const { hls, video, onStuck } = start();
    hls.fire('hlsLevelUpdated');
    hls.fire('hlsFragBuffered', liveFragment);
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

  it('seeks only into the window the origin serves now, never into the old buffer', () => {
    const { hls, video } = start();
    hls.liveSyncPosition = 40;
    hls.latestLevelDetails = { fragmentStart: 28 };
    video.setBuffered([[8, 16]]); // only video from before the pause
    hls.fire('hlsLevelUpdated');
    hls.fire('hlsFragBuffered');
    expect(video.seeks).toEqual([]);
    video.setBuffered([[8, 16], [30, 42]]);
    hls.fire('hlsFragBuffered');
    expect(video.seeks).toEqual([40]);
  });

  it('waits while hls.js has no level details', () => {
    const { hls, video } = start();
    hls.liveSyncPosition = 20;
    hls.latestLevelDetails = null;
    video.setBuffered([[8, 60]]);
    hls.fire('hlsLevelUpdated');
    expect(video.seeks).toEqual([]);
  });

  it('is stuck at the window when no new live data arrives, whatever the old buffer does', () => {
    const { hls, video, onStuck } = start();
    hls.latestLevelDetails = { fragmentStart: 80 };
    video.advance(100);
    video.setBuffered([[80, 106]]);
    video.emit('playing');
    vi.advanceTimersByTime(RESYNC_WINDOW_MS - 1);
    expect(onStuck).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(onStuck).toHaveBeenCalledTimes(1);
    expect(hls.count('hlsLevelUpdated')).toBe(0);
    expect(hls.count('hlsFragBuffered')).toBe(0);
    expect(video.count('timeupdate')).toBe(0);
  });

  it('is stuck at the window when a reload arrives but no fragment inside it', () => {
    const { hls, video, onStuck } = start();
    hls.latestLevelDetails = { fragmentStart: 200 };
    video.advance(100);
    video.emit('playing');
    hls.fire('hlsLevelUpdated');
    vi.advanceTimersByTime(RESYNC_WINDOW_MS);
    expect(onStuck).toHaveBeenCalledTimes(1);
  });

  it('is stuck when the only fragment buffered lies before the served window', () => {
    const { hls, video, onStuck } = start();
    hls.latestLevelDetails = { fragmentStart: 200 };
    video.advance(100);
    video.emit('playing');
    hls.fire('hlsLevelUpdated');
    hls.fire('hlsFragBuffered', { frag: { type: 'main', start: 150, duration: 2 } });
    vi.advanceTimersByTime(RESYNC_WINDOW_MS);
    expect(onStuck).toHaveBeenCalledTimes(1);
  });

  it('never seeks on a fragment that arrives before any reload', () => {
    const { hls, video } = start();
    hls.liveSyncPosition = 20;
    video.setBuffered([[8, 60]]);
    hls.fire('hlsFragBuffered', liveFragment);
    expect(video.seeks).toEqual([]);
    hls.fire('hlsLevelUpdated');
    expect(video.seeks).toEqual([20]);
  });

  it('is not stuck once new live data is buffered, the seek lands and the video plays on', () => {
    const { hls, video, onStuck } = start();
    hls.liveSyncPosition = 232;
    hls.latestLevelDetails = { fragmentStart: 200 };
    video.setBuffered([[8, 16]]);
    hls.fire('hlsLevelUpdated');
    expect(video.seeks).toEqual([]);
    video.setBuffered([[8, 16], [230, 240]]);
    hls.fire('hlsFragBuffered', { frag: { type: 'main', start: 230, duration: 2 } });
    expect(video.seeks).toEqual([232]);
    video.advance(236);
    vi.advanceTimersByTime(RESYNC_WINDOW_MS * 3);
    expect(onStuck).not.toHaveBeenCalled();
  });

  it('is not stuck once a reload lands and the playhead moves into the window', () => {
    const { hls, video, onStuck } = start();
    hls.liveSyncPosition = 40;
    hls.latestLevelDetails = { fragmentStart: 28 };
    video.setBuffered([[8, 16], [30, 42]]);
    hls.fire('hlsLevelUpdated');
    expect(video.seeks).toEqual([40]);
    video.advance(44);
    vi.advanceTimersByTime(RESYNC_WINDOW_MS * 3);
    expect(onStuck).not.toHaveBeenCalled();
  });

  it('reports a freeze after the seek one window after the seek', () => {
    const { hls, video, onStuck } = start();
    hls.liveSyncPosition = 40;
    hls.latestLevelDetails = { fragmentStart: 28 };
    video.setBuffered([[30, 42]]);
    video.advance(11); // progress before the seek does not count after it
    vi.advanceTimersByTime(3000);
    hls.fire('hlsLevelUpdated');
    expect(video.seeks).toEqual([40]);
    vi.advanceTimersByTime(RESYNC_WINDOW_MS - 1);
    expect(onStuck).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(onStuck).toHaveBeenCalledTimes(1);
  });

  it('reports a freeze after a seek made by anyone, one window after that seek', () => {
    const { hls, video, onStuck } = start();
    hls.latestLevelDetails = { fragmentStart: 80 };
    video.advance(100);
    video.setBuffered([[80, 106]]);
    video.emit('playing');
    hls.latestLevelDetails = { fragmentStart: 200 };
    hls.fire('hlsLevelUpdated');
    hls.fire('hlsFragBuffered', { frag: { type: 'main', start: 228, duration: 2 } });
    vi.advanceTimersByTime(3000);
    video.advance(230);
    video.emit('seeking');
    vi.advanceTimersByTime(RESYNC_WINDOW_MS - 1);
    expect(onStuck).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(onStuck).toHaveBeenCalledTimes(1);
    expect(video.count('seeking')).toBe(0);
  });

  it('is not stuck when a seek by anyone lands and the video plays on', () => {
    const { hls, video, onStuck } = start();
    hls.latestLevelDetails = { fragmentStart: 80 };
    video.advance(100);
    video.setBuffered([[80, 106]]);
    video.emit('playing');
    hls.latestLevelDetails = { fragmentStart: 200 };
    hls.fire('hlsLevelUpdated');
    video.advance(230);
    video.emit('seeking');
    hls.fire('hlsFragBuffered', { frag: { type: 'main', start: 228, duration: 2 } });
    video.emit('playing');
    video.advance(234);
    vi.advanceTimersByTime(RESYNC_WINDOW_MS * 3);
    expect(onStuck).not.toHaveBeenCalled();
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
    expect(video.count('timeupdate')).toBe(0);
    expect(video.count('seeking')).toBe(0);
  });
});
