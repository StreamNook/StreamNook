import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { FIRST_MEDIA_CHECK_MS, watchFirstMedia, watchStalls, watchWedge, wedgedOnBuffer } from './stallWatchdog';

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

describe('wedgedOnBuffer', () => {
  /** A media element at `currentTime` holding the given buffered ranges. */
  const media = (over: Partial<{ paused: boolean; readyState: number; currentTime: number }>, ranges: [number, number][]) => ({
    paused: false,
    readyState: 1,
    currentTime: 0.955,
    ...over,
    buffered: {
      length: ranges.length,
      start: (i: number) => ranges[i][0],
      end: (i: number) => ranges[i][1],
    } as TimeRanges,
  });

  it('reports the captured start: playing, at HAVE_METADATA, with seconds buffered over the playhead', () => {
    expect(wedgedOnBuffer(media({}, [[0.044, 6.146]]))).toBe(true);
    expect(wedgedOnBuffer(media({ currentTime: 8.487 }, [[0.044, 12.268]]))).toBe(true);
  });

  it('leaves an element that can show its frame alone', () => {
    expect(wedgedOnBuffer(media({ readyState: 2 }, [[0.044, 6.146]]))).toBe(false);
    expect(wedgedOnBuffer(media({ readyState: 4 }, [[0.044, 6.146]]))).toBe(false);
  });

  it('leaves a paused element alone', () => {
    expect(wedgedOnBuffer(media({ paused: true }, [[0.044, 6.146]]))).toBe(false);
  });

  it('is not a wedge while the media at the playhead is still arriving', () => {
    expect(wedgedOnBuffer(media({}, []))).toBe(false);
    expect(wedgedOnBuffer(media({}, [[0.044, 1.5]]))).toBe(false);
    expect(wedgedOnBuffer(media({}, [[2, 8]]))).toBe(false);
  });
});

describe('watchWedge', () => {
  /** A playing element at HAVE_METADATA (or `readyState`) at `ct`, with media buffered past it. */
  const wedgedAt = (ct: number, readyState = 1) => ({
    paused: false,
    readyState,
    currentTime: ct,
    buffered: { length: 1, start: () => 0, end: () => ct + 6 } as unknown as TimeRanges,
  });

  it('reloads only after 4s wedged with a still playhead', () => {
    const tick = watchWedge();
    expect(tick(wedgedAt(1), 0)).toEqual({ wedged: true, reload: false });
    expect(tick(wedgedAt(1), 2000)).toEqual({ wedged: true, reload: false });
    expect(tick(wedgedAt(1), 3999)).toEqual({ wedged: true, reload: false });
    expect(tick(wedgedAt(1), 4000)).toEqual({ wedged: true, reload: true });
  });

  it('a playhead that moves is timed afresh', () => {
    const tick = watchWedge();
    tick(wedgedAt(1), 0);
    expect(tick(wedgedAt(1.5), 2000).reload).toBe(false);
    expect(tick(wedgedAt(1.5), 4000).reload).toBe(false);
    expect(tick(wedgedAt(1.5), 6000).reload).toBe(true);
  });

  it('starts over when the element recovers', () => {
    const tick = watchWedge();
    tick(wedgedAt(1), 0);
    expect(tick(wedgedAt(1, 4), 2000)).toEqual({ wedged: false, reload: false });
    expect(tick(wedgedAt(1), 4000).reload).toBe(false);
    expect(tick(wedgedAt(1), 8000).reload).toBe(true);
  });

  it('times a new wedge after a reload', () => {
    const tick = watchWedge();
    tick(wedgedAt(1), 0);
    expect(tick(wedgedAt(1), 4000).reload).toBe(true);
    expect(tick(wedgedAt(1), 6000).reload).toBe(false);
    expect(tick(wedgedAt(1), 8000).reload).toBe(false);
    expect(tick(wedgedAt(1), 10000).reload).toBe(true);
  });
});

describe('watchFirstMedia', () => {
  /**
   * A hand-driven clock and scheduler, a loader whose in-flight state the test
   * sets (already downloading when the watch arms, if `inFlightAtArm`), and a
   * reload that answers from `answers`.
   */
  function harness(answers: boolean[] = [], inFlightAtArm = false) {
    let t = 0;
    let pending: (() => void) | null = null;
    let dueAt = 0;
    let current = true;
    let finished = 0;
    let inFlight = inFlightAtArm;
    const reloads: string[] = [];
    const watch = watchFirstMedia({
      now: () => t,
      inFlight: () => inFlight,
      schedule: (fn, ms) => {
        pending = fn;
        dueAt = t + ms;
      },
      isCurrent: () => current,
      reload: (why) => {
        reloads.push(why);
        return answers.shift() ?? true;
      },
      finished: () => {
        finished += 1;
      },
    });
    /** Move the clock to `to`, running every check due on the way. */
    const runTo = (to: number) => {
      while (pending && dueAt <= to) {
        t = dueAt;
        const fn = pending;
        pending = null;
        fn();
      }
      t = to;
    };
    return {
      watch,
      runTo,
      reloads,
      finished: () => finished,
      scheduled: () => pending !== null,
      replace: () => {
        current = false;
      },
      setInFlight: (v: boolean) => {
        inFlight = v;
      },
    };
  }

  it('reloads after 5s with nothing in flight', () => {
    const h = harness();
    h.runTo(4000);
    expect(h.reloads).toHaveLength(0);
    h.runTo(5000);
    expect(h.reloads).toHaveLength(1);
    expect(h.reloads[0]).toContain('(idle)');
    expect(h.finished()).toBe(1);
    expect(h.scheduled()).toBe(false);
  });

  it('waits for a download in flight until the 20s cap', () => {
    const h = harness();
    h.runTo(500);
    h.setInFlight(true);
    h.runTo(19000);
    expect(h.reloads).toHaveLength(0);
    h.runTo(20000);
    expect(h.reloads).toHaveLength(1);
    expect(h.reloads[0]).toContain('(cap)');
  });

  it('a download already in flight when the wait arms is waited out to the cap', () => {
    const h = harness([], true);
    h.runTo(19000);
    expect(h.reloads).toHaveLength(0);
    h.runTo(20000);
    expect(h.reloads).toHaveLength(1);
    expect(h.reloads[0]).toContain('(cap)');
  });

  it('a download that ends starts the idle time from its progress', () => {
    const h = harness([], true);
    h.runTo(3000);
    h.setInFlight(false);
    h.watch.progress();
    h.runTo(7000);
    expect(h.reloads).toHaveLength(0);
    h.runTo(8000);
    expect(h.reloads).toHaveLength(1);
    expect(h.reloads[0]).toContain('(idle)');
  });

  it('asks again after a refused reload', () => {
    const h = harness([false, true], true);
    h.runTo(20000);
    expect(h.reloads).toHaveLength(1);
    expect(h.finished()).toBe(0);
    h.runTo(20000 + FIRST_MEDIA_CHECK_MS);
    expect(h.reloads).toHaveLength(2);
    expect(h.finished()).toBe(1);
  });

  it('the first media ends the wait and later checks do nothing', () => {
    const h = harness();
    h.runTo(1500);
    expect(h.watch.media()).toBe(true);
    expect(h.watch.media()).toBe(false);
    h.runTo(30000);
    expect(h.reloads).toHaveLength(0);
    expect(h.finished()).toBe(1);
  });

  it('stops when the player is replaced', () => {
    const h = harness();
    h.runTo(1500);
    h.replace();
    h.runTo(30000);
    expect(h.reloads).toHaveLength(0);
    expect(h.finished()).toBe(1);
  });
});

describe('the player and the store wire them', () => {
  const root = resolve(__dirname, '..', '..');
  const read = (path: string) => readFileSync(resolve(root, path), 'utf8').replace(/\r\n/g, '\n');
  const player = read('src/components/VideoPlayer.tsx');
  const store = read('src/stores/AppStore.ts');
  /** The text from `start` up to the first `end` after it. */
  const slice = (src: string, start: string, end: string) => {
    const a = src.indexOf(start);
    expect(a, start).toBeGreaterThan(-1);
    const b = src.indexOf(end, a + start.length);
    expect(b, end).toBeGreaterThan(-1);
    return src.slice(a, b);
  };

  it('times the wedge once per player, on macOS, and reloads with its own reason', () => {
    const dog = slice(player, 'let lastRecover = 0;', 'if (!v || v.paused || v.seeking) {');
    expect(dog).toContain('const wedge = watchWedge();');
    expect(dog.indexOf('const wedge = watchWedge();')).toBeLessThan(
      dog.indexOf('behindLiveWatchdogRef.current = setInterval('),
    );
    expect(dog).toContain('if (IS_MAC && v) {');
    expect(dog).toContain('const w = wedge(v, performance.now());');
    expect(dog).toContain("'wedge',");
    expect(dog).not.toContain('let wedged = 0');
  });

  it('starts at once off macOS, and on macOS parks without placing when nothing should play', () => {
    const now = slice(player, 'if (!IS_MAC) {', 'return;');
    expect(now.indexOf('placeLL();')).toBeLessThan(now.indexOf('playOrPark();'));
    const quiet = slice(player, 'if (!wantsPlay) {', '// A start that buffers nothing reloads');
    expect(quiet).toContain('playOrPark();');
    expect(quiet).toContain('hls.once(Hls.Events.FRAG_BUFFERED, placeLL);');
    expect(quiet).not.toContain('placeLL();');
  });

  it('a macOS start that buffers nothing reloads through the recovery', () => {
    const wait = slice(player, '// A start that buffers nothing reloads', '\n          });\n');
    expect(wait).toContain('const first = watchFirstMedia({');
    expect(wait).toContain('inFlight: () => fragLoad.inFlight,');
    expect(wait).toContain("return recover(why, true, 'no-buffer');");
    expect(wait).toContain('isCurrent: () => hlsRef.current === hls,');
    expect(wait).toContain('bufferGateTimeoutRef.current = setTimeout(fn, ms);');
    expect(wait).toContain('finished: () => unhook(),');
    for (const [event, handler] of [
      ['FRAG_LOADING', 'onProgress'],
      ['FRAG_LOADED', 'onProgress'],
      ['FRAG_PARSED', 'onProgress'],
      ['FRAG_BUFFERED', 'onMedia'],
    ]) {
      expect(wait).toContain(`hls.on(Hls.Events.${event}, ${handler});`);
      expect(wait).toContain(`hls.off(Hls.Events.${event}, ${handler});`);
    }
    const media = slice(wait, 'const onMedia = () => {', '};');
    const order = ['if (!first.media()) return;', 'placeLL();', 'playOrPark();'].map((s) => media.indexOf(s));
    expect(order.every((i) => i > -1)).toBe(true);
    expect(order).toEqual([...order].sort((a, b) => a - b));
  });

  it('a viewer who paused while it waited is not reloaded', () => {
    const reload = slice(player, 'reload: (why) => {', 'finished: () => unhook(),');
    expect(reload.indexOf("if (heldPause('main', liveKeyNow())) {")).toBeGreaterThan(-1);
    expect(reload.indexOf("if (heldPause('main', liveKeyNow())) {")).toBeLessThan(
      reload.indexOf("return recover(why, true, 'no-buffer');"),
    );
  });

  it('macOS starts only when nothing holds them, and tracks downloads before any load', () => {
    expect(player).toContain(
      "const wantsPlay =\n              currentSettings.autoplay && !heldPause('main', liveKeyNow()) && !park?.loadStopped;",
    );
    const tracker = player.indexOf('const fragLoad = { inFlight: false };');
    expect(tracker).toBeGreaterThan(-1);
    expect(tracker).toBeLessThan(player.indexOf('hls.on(Hls.Events.MANIFEST_PARSED'));
    const hooks = slice(player, 'const fragLoad = { inFlight: false };', 'if (isLowLatencyChannel) {');
    expect(hooks).toContain('if (IS_MAC && isLowLatencyChannel) {');
    expect(hooks).toContain('fragLoad.inFlight = true;');
    expect(hooks).toContain('if (!data.part) fragLoad.inFlight = false;');
  });

  it('counts a reload after the restart is claimed and still wanted', () => {
    expect(store.indexOf('async function countReload(')).toBeGreaterThan(-1);
    expect(store.indexOf('async function countReload(')).toBeLessThan(store.indexOf('restartStream: async (options) => {'));
    const restart = slice(store, 'restartStream: async (options) => {', '\n  },\n');
    expect(restart).not.toContain('note_playback_reload');
    const provider = slice(restart, "if (provider !== 'twitch') {", "await invoke('stop_stream');");
    expect(provider.indexOf('options.ifUrl')).toBeLessThan(provider.indexOf('await countReload(channel, options?.reason);'));
    const twitch = slice(restart, 'await settleTransientSwap();', "await invoke('stop_stream');");
    expect(twitch.indexOf('options.ifUrl')).toBeLessThan(twitch.indexOf('await countReload(channel, options?.reason);'));
    expect(restart.indexOf('restartInFlight = running;')).toBeLessThan(restart.indexOf('await countReload('));
  });

  it('only the viewer quality change ends a codec hold', () => {
    const transient = slice(store, 'applyTransientQuality: async (quality: string) => {', 'changeStreamQuality: async (quality: string) => {');
    expect(transient).not.toContain("'change_stream_quality'");
    expect(transient).toContain("invoke<StreamStartResult>('start_stream', {");
    const viewer = slice(store, 'changeStreamQuality: async (quality: string) => {', 'startStream: async (');
    expect(viewer).toContain("'change_stream_quality'");
  });
});
