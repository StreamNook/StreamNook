import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  PAUSE_PARK_OVERRIDE_KEY,
  RESYNC_AFTER_PAUSE_MS,
  STOP_LOAD_AFTER_PAUSE_MS,
  clearPauseIntent,
  clearTileOnEnd,
  createPausePark,
  forgetOwnChannel,
  forgetTile,
  heldPause,
  heldTileIds,
  holdOwnChannel,
  isTileHeld,
  markTileHeld,
  notePauseIntent,
  pauseIntentOf,
  pauseParkTimings,
  pausedTileIds,
  resyncCutoffMs,
  tileStreamEnded,
  windowedPauseTimings,
  type PauseParkTimings,
} from './pausePark';

describe('pausePark', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  const make = (paused = true, timings?: () => PauseParkTimings) => {
    const calls: string[] = [];
    let t = 0;
    const park = createPausePark({
      stopLoad: () => calls.push('stop'),
      startLoad: () => calls.push('start'),
      stillPaused: () => paused,
      now: () => t,
      timings,
    });
    return { park, calls, at: (ms: number) => (t = ms) };
  };

  it('stops loading only after the grace and resumes on a short pause', () => {
    const { park, calls, at } = make();
    park.paused();
    vi.advanceTimersByTime(STOP_LOAD_AFTER_PAUSE_MS - 1);
    expect(calls).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(calls).toEqual(['stop']);
    at(5_000);
    expect(park.play()).toBe('resume');
    expect(calls).toEqual(['stop', 'start']);
  });

  it('a play inside the grace changes nothing', () => {
    const { park, calls, at } = make();
    park.paused();
    at(500);
    expect(park.play()).toBe('none');
    vi.advanceTimersByTime(STOP_LOAD_AFTER_PAUSE_MS);
    expect(calls).toEqual([]);
  });

  it('a long pause restarts loading and asks the caller for the live edge', () => {
    const { park, calls, at } = make();
    park.paused();
    vi.advanceTimersByTime(STOP_LOAD_AFTER_PAUSE_MS);
    at(RESYNC_AFTER_PAUSE_MS);
    expect(park.play()).toBe('resync');
    expect(calls).toEqual(['stop', 'start']);
  });

  describe('with a resync load', () => {
    const makeWith = () => {
      const calls: string[] = [];
      let t = 0;
      const park = createPausePark({
        stopLoad: () => calls.push('stop'),
        startLoad: () => calls.push('start'),
        resyncLoad: () => calls.push('resync'),
        stillPaused: () => true,
        now: () => t,
      });
      return { park, calls, at: (ms: number) => (t = ms) };
    };

    it('a long pause hands the load to resyncLoad once and never calls startLoad', () => {
      const { park, calls, at } = makeWith();
      park.paused();
      vi.advanceTimersByTime(STOP_LOAD_AFTER_PAUSE_MS);
      at(RESYNC_AFTER_PAUSE_MS);
      expect(park.play()).toBe('resync');
      expect(calls).toEqual(['stop', 'resync']);
      expect(park.loadStopped).toBe(false);
    });

    it('stops a load that is still running before resyncLoad', () => {
      const { park, calls, at } = makeWith();
      park.paused();
      at(RESYNC_AFTER_PAUSE_MS);
      expect(park.play()).toBe('resync');
      expect(calls).toEqual(['stop', 'resync']);
    });

    it('a pause under the cut-off still resumes with startLoad', () => {
      const { park, calls, at } = makeWith();
      park.paused();
      vi.advanceTimersByTime(STOP_LOAD_AFTER_PAUSE_MS);
      at(RESYNC_AFTER_PAUSE_MS - 1);
      expect(park.play()).toBe('resume');
      expect(calls).toEqual(['stop', 'start']);
    });

    it('a play inside the grace changes nothing', () => {
      const { park, calls, at } = makeWith();
      park.paused();
      at(500);
      expect(park.play()).toBe('none');
      expect(calls).toEqual([]);
    });
  });

  it('a held pause from before a rebuild stops at once and keeps its time', () => {
    const { park, calls, at } = make();
    at(10_000);
    park.paused(1_000);
    vi.advanceTimersByTime(0);
    expect(calls).toEqual(['stop']);
    at(1_000 + RESYNC_AFTER_PAUSE_MS);
    expect(park.play()).toBe('resync');
    expect(calls).toEqual(['stop', 'start']);
  });

  it('a replaced or playing instance never stops loading', () => {
    const { park, calls } = make(false);
    park.paused();
    vi.advanceTimersByTime(STOP_LOAD_AFTER_PAUSE_MS);
    expect(calls).toEqual([]);
  });

  it('dispose cancels the timer', () => {
    const { park, calls } = make();
    park.paused();
    park.dispose();
    vi.advanceTimersByTime(STOP_LOAD_AFTER_PAUSE_MS);
    expect(calls).toEqual([]);
    expect(park.play()).toBe('none');
  });

  it('a test run can shorten both spans', () => {
    const { park, calls, at } = make(true, () => ({ stopLoadMs: 50, resyncMs: 100 }));
    park.paused();
    vi.advanceTimersByTime(50);
    expect(calls).toEqual(['stop']);
    at(100);
    expect(park.play()).toBe('resync');
  });

  it('reads the override key, and ignores what is not a positive number', () => {
    expect(pauseParkTimings()).toEqual({ stopLoadMs: STOP_LOAD_AFTER_PAUSE_MS, resyncMs: RESYNC_AFTER_PAUSE_MS });
    let stored: string | null = JSON.stringify({ stopLoadMs: 3000, resyncMs: 5000 });
    vi.stubGlobal('localStorage', { getItem: (k: string) => (k === PAUSE_PARK_OVERRIDE_KEY ? stored : null) });
    expect(pauseParkTimings()).toEqual({ stopLoadMs: 3000, resyncMs: 5000 });
    stored = JSON.stringify({ stopLoadMs: -1, resyncMs: 'x' });
    expect(pauseParkTimings()).toEqual({ stopLoadMs: STOP_LOAD_AFTER_PAUSE_MS, resyncMs: RESYNC_AFTER_PAUSE_MS });
    stored = 'not json';
    expect(pauseParkTimings()).toEqual({ stopLoadMs: STOP_LOAD_AFTER_PAUSE_MS, resyncMs: RESYNC_AFTER_PAUSE_MS });
  });

  it('the resync cut-off is the time until the window start passes the buffer end', () => {
    expect(resyncCutoffMs(102, 90, 0)).toBe(12_000);
    expect(resyncCutoffMs(102, 90, 2)).toBe(10_000);
    expect(resyncCutoffMs(200, 90, 0)).toBe(RESYNC_AFTER_PAUSE_MS);
    expect(resyncCutoffMs(100, 130, 0)).toBe(0);
    expect(resyncCutoffMs(102, null, 0)).toBe(RESYNC_AFTER_PAUSE_MS);
    expect(resyncCutoffMs(102, undefined, 0)).toBe(RESYNC_AFTER_PAUSE_MS);
  });

  it('a buffer that ends before the window starts resyncs at once', () => {
    const hls = { latestLevelDetails: { fragmentStart: 130, age: 0 } };
    const video = { currentTime: 100, buffered: { length: 1, start: () => 90, end: () => 102 } };
    const { park, at } = make(true, () => windowedPauseTimings(hls, video as never));
    park.paused(0);
    at(500);
    expect(park.play(500)).toBe('resync');
  });

  it('a playhead far behind with a buffer to the edge resumes a short pause in place', () => {
    // 20 s behind the edge, the buffer reaches the edge, the window is 12 s.
    const hls = { latestLevelDetails: { fragmentStart: 108, age: 0 } };
    const video = { currentTime: 100, buffered: { length: 1, start: () => 90, end: () => 120 } };
    const { park, calls, at } = make(true, () => ({ ...windowedPauseTimings(hls, video as never), stopLoadMs: 100 }));
    park.paused(0);
    vi.advanceTimersByTime(100);
    expect(calls).toEqual(['stop']);
    at(500);
    expect(park.play(500)).toBe('resume');
    expect(calls).toEqual(['stop', 'start']);
  });

  it('the cut-off is read when the pause starts, not when play comes', () => {
    let fragmentStart = 90;
    const hls = {
      get latestLevelDetails() {
        return { fragmentStart, age: 0 };
      },
    };
    const video = { currentTime: 100, buffered: { length: 1, start: () => 90, end: () => 102 } };
    const { park, at } = make(true, () => windowedPauseTimings(hls, video as never));
    park.paused(0);
    fragmentStart = 130; // the window rolled on during the pause
    at(11_000);
    expect(park.play(11_000)).toBe('none'); // 12 s cut-off from the pause start
  });

  it('no playlist details keep the default cut-off', () => {
    const video = { currentTime: 100, buffered: { length: 0, start: () => 0, end: () => 0 } };
    expect(windowedPauseTimings({}, video as never).resyncMs).toBe(RESYNC_AFTER_PAUSE_MS);
    expect(windowedPauseTimings({ latestLevelDetails: null }, video as never).resyncMs).toBe(RESYNC_AFTER_PAUSE_MS);
  });

  it('a tile hold survives a remount and ends when the tile is forgotten', () => {
    forgetTile('tile-9');
    expect(isTileHeld('tile-9')).toBe(false);
    markTileHeld('tile-9');
    expect(isTileHeld('tile-9')).toBe(true); // docked and undocked: the same tile
    expect(isTileHeld('tile-8')).toBe(false);
    forgetTile('tile-9');
    expect(isTileHeld('tile-9')).toBe(false);
  });

  it('a tile whose stream ended loses its pause and its own-channel hold', () => {
    markTileHeld('tile-5');
    notePauseIntent('tile-5', 'tile-5', 4);
    expect(heldPause('tile-5', 'tile-5')).not.toBeNull();
    clearTileOnEnd('tile-5');
    expect(heldPause('tile-5', 'tile-5')).toBeNull();
    expect(isTileHeld('tile-5')).toBe(false);
  });

  it('only a tile that is offline and failed has ended; a restarting tile keeps its pause', () => {
    expect(tileStreamEnded({ offline: true, loadError: true })).toBe(true);
    expect(tileStreamEnded({ offline: true, loadError: false })).toBe(false); // retrying
    expect(tileStreamEnded({ offline: true })).toBe(false);
    expect(tileStreamEnded({ offline: false, loadError: true })).toBe(false);
    expect(tileStreamEnded({})).toBe(false);
    expect(tileStreamEnded(undefined)).toBe(false);
  });

  it('lists the paused tiles and the held tiles for a snapshot', () => {
    clearPauseIntent('main');
    notePauseIntent('main', 'twitch:a');
    notePauseIntent('tile-p', 'tile-p');
    markTileHeld('tile-q');
    expect(pausedTileIds()).toEqual(['tile-p']); // the main player is not a tile
    expect(heldTileIds()).toEqual(['tile-q']);
    clearPauseIntent('main');
    clearPauseIntent('tile-p');
    forgetTile('tile-q');
    expect(pausedTileIds()).toEqual([]);
    expect(heldTileIds()).toEqual([]);
  });

  it('a pause intent holds for its channel only', () => {
    clearPauseIntent('main');
    notePauseIntent('main', 'twitch:a', 7);
    expect(heldPause('main', 'twitch:a')).toEqual({ key: 'twitch:a', at: 7 });
    notePauseIntent('main', 'twitch:a', 9);
    expect(heldPause('main', 'twitch:a')?.at).toBe(7);
    expect(heldPause('main', 'twitch:b')).toBeNull();
    expect(heldPause('main', 'twitch:a')).toBeNull();
    notePauseIntent('tile-1', 'tile-1');
    clearPauseIntent('tile-1');
    expect(heldPause('tile-1', 'tile-1')).toBeNull();
  });

  it('nothing on screen reads no pause and drops none', () => {
    clearPauseIntent('main');
    notePauseIntent('main', 'twitch:a', 3);
    expect(heldPause('main', null)).toBeNull();
    expect(pauseIntentOf('main')).toEqual({ key: 'twitch:a', at: 3 });
    expect(heldPause('main', 'twitch:a')?.at).toBe(3);
    clearPauseIntent('main');
    expect(pauseIntentOf('main')).toBeNull();
  });

  it('the own channel opens paused once per opening', () => {
    clearPauseIntent('main');
    forgetOwnChannel();
    expect(holdOwnChannel('twitch:me', 5)).toBe(true);
    expect(heldPause('main', 'twitch:me')?.at).toBe(5);
    clearPauseIntent('main'); // the viewer pressed play
    expect(holdOwnChannel('twitch:me')).toBe(false); // a restart of the same opening
    expect(heldPause('main', 'twitch:me')).toBeNull();
    forgetOwnChannel(); // stopped, or another channel opened
    expect(holdOwnChannel('twitch:me', 8)).toBe(true);
    expect(heldPause('main', 'twitch:me')?.at).toBe(8);
    clearPauseIntent('main');
    forgetOwnChannel();
  });
});
