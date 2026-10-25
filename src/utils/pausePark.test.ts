import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  PAUSE_PARK_OVERRIDE_KEY,
  RESYNC_AFTER_PAUSE_MS,
  STOP_LOAD_AFTER_PAUSE_MS,
  clearPauseIntent,
  createPausePark,
  forgetOwnChannel,
  heldPause,
  holdOwnChannel,
  notePauseIntent,
  pauseIntentOf,
  pauseParkTimings,
  resyncCutoffMs,
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

  it('the resync cut-off fits the window the origin serves', () => {
    expect(resyncCutoffMs(12, 2)).toBe(10_000);
    expect(resyncCutoffMs(12, 2)).toBeLessThan(12_000);
    expect(resyncCutoffMs(60, 2)).toBe(RESYNC_AFTER_PAUSE_MS);
    expect(resyncCutoffMs(12, 20)).toBe(0);
    expect(resyncCutoffMs(null, 2)).toBe(RESYNC_AFTER_PAUSE_MS);
    expect(resyncCutoffMs(undefined, 2)).toBe(RESYNC_AFTER_PAUSE_MS);
  });

  it('a short window resyncs a pause the default span would resume', () => {
    const hls = { latestLevelDetails: { totalduration: 12 } };
    const video = { currentTime: 100, buffered: { length: 1, start: () => 90, end: () => 102 } };
    const { park, at } = make(true, () => windowedPauseTimings(hls, video as never));
    park.paused(0);
    at(11_000);
    expect(park.play(11_000)).toBe('resync');
  });

  it('no playlist details keep the default cut-off', () => {
    const video = { currentTime: 100, buffered: { length: 0, start: () => 0, end: () => 0 } };
    expect(windowedPauseTimings({}, video as never).resyncMs).toBe(RESYNC_AFTER_PAUSE_MS);
    expect(windowedPauseTimings({ latestLevelDetails: null }, video as never).resyncMs).toBe(RESYNC_AFTER_PAUSE_MS);
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
